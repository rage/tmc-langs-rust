//! Create clap app

use anyhow::Context;
use clap::Parser;
use schemars::JsonSchema;
use std::{path::PathBuf, str::FromStr};
use tmc_langs::{
    CombinedCourseData, Compression, CourseData, CourseDetails, CourseExercise,
    DownloadOrUpdateCourseExercisesResult, ExerciseDesc, ExerciseDetails,
    ExercisePackagingConfiguration, Language, LocalExercise, NewSubmission, Organization, Review,
    RunResult, StyleValidationResult, Submission, SubmissionFeedbackResponse, SubmissionFinished,
    UpdateResult, UpdatedExercise,
};
// use tmc_langs_util::task_executor::RefreshData;

#[derive(Parser)]
#[clap(
    version,
    author,
    about,
    subcommand_required(true),
    arg_required_else_help(true)
)]
pub struct Opt {
    /// Pretty-prints all output
    #[clap(long, short)]
    pub pretty: bool,
    /// Name used to differentiate between different TMC clients.
    #[clap(long, short)]
    pub client_name: Option<String>,
    /// Client version.
    #[clap(long, short = 'v')]
    pub client_version: Option<String>,
    #[clap(subcommand)]
    pub cmd: Command,
}

#[derive(Parser)]
pub enum Command {
    /// Checks the code style for the given exercise
    #[clap(long_about = schema_leaked::<Option<StyleValidationResult>>())]
    Checkstyle {
        /// Path to the directory where the project resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// Locale as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.
        #[clap(long)]
        locale: Locale,
        /// If defined, the check results will be written to this path. Overwritten if it already exists.
        #[clap(long)]
        output_path: Option<PathBuf>,
    },

    /// Cleans the target exercise using the appropriate language plugin
    #[clap(long_about = SCHEMA_NULL)]
    Clean {
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
    },

    /// Compresses the target exercise. Only includes student files using the student file policy of the exercise's plugin
    #[clap(long_about = SCHEMA_NULL)]
    CompressProject {
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// Path to the output archive. Overwritten if it already exists.
        #[clap(long)]
        output_path: PathBuf,
        /// Compression algorithm to use.
        #[clap(long, default_value_t = Compression::Zip)]
        compression: Compression,
        /// If set, simply compresses the target directory with all of its files.
        #[clap(long)]
        naive: bool,
    },

    #[clap(subcommand)]
    Core(Core),

    /// Extracts an exercise from an archive. If the output-path is a project root, the plugin's student file policy will be used to avoid overwriting student files
    #[clap(long_about = SCHEMA_NULL)]
    ExtractProject {
        /// Path to the archive.
        #[clap(long)]
        archive_path: PathBuf,
        /// Path to the directory where the archive will be extracted.
        #[clap(long)]
        output_path: PathBuf,
        /// Compression algorithm used for the archive.
        #[clap(long, default_value_t = Compression::Zip)]
        compression: Compression,
        /// If set, simply extracts the target directory with all of its files.
        #[clap(long)]
        naive: bool,
    },

    /// Parses @Points notations from an exercise's exercise files and returns the point names found
    #[clap(long_about = schema_leaked::<Vec<String>>())]
    FastAvailablePoints {
        /// Path to the directory where the projects reside.
        #[clap(long)]
        exercise_path: PathBuf,
    },

    /// Finds all exercise root directories inside the exercise-path
    #[clap(long_about = schema_leaked::<Vec<PathBuf>>())]
    FindExercises {
        /// Path to the directory where the projects reside.
        #[clap(long)]
        exercise_path: PathBuf,
        /// If given, the search results will be written to this path. Overwritten if it already exists.
        #[clap(long)]
        output_path: Option<PathBuf>,
    },

    /// Returns a configuration which separately lists the student files and exercise files inside the given exercise
    #[clap(long_about = schema_leaked::<ExercisePackagingConfiguration>())]
    GetExercisePackagingConfiguration {
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// If given, the configuration will be written to this path. Overwritten if it already exists.
        #[clap(long)]
        output_path: Option<PathBuf>,
    },

    /// Returns a list of local exercises for the given course
    #[clap(long_about = schema_leaked::<Vec<LocalExercise>>())]
    ListLocalCourseExercises {
        /// The course slug the local exercises of which should be listed.
        #[clap(long)]
        course_slug: String,
    },

    /// Processes the exercise files in exercise-path, removing all code marked as stubs
    #[clap(long_about = SCHEMA_NULL)]
    PrepareSolution {
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// Path to the directory where the processed files will be written.
        #[clap(long)]
        output_path: PathBuf,
    },

    /// Processes the exercise files in exercise-path, removing all code marked as solutions
    #[clap(long_about = SCHEMA_NULL)]
    PrepareStub {
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// Path to the directory where the processed files will be written.
        #[clap(long)]
        output_path: PathBuf,
    },

    /// Takes a submission archive and turns it into an archive with reset test files, and tmc-params, ready for further processing
    #[clap(long_about = SCHEMA_NULL)]
    PrepareSubmission {
        /// The output format of the submission archive. Defaults to tar.
        #[clap(long, default_value_t = Compression::Tar)]
        output_format: Compression,
        /// Path to exercise's clone path, where the unmodified test files will be copied from.
        #[clap(long)]
        clone_path: PathBuf,
        /// Path to the resulting archive. Overwritten if it already exists.
        #[clap(long)]
        output_path: PathBuf,
        /// If given, the tests will be copied from this stub instead, effectively ignoring hidden tests.
        // alias for backwards compatibility
        #[clap(long, alias = "stub-zip-path")]
        stub_archive_path: Option<PathBuf>,
        /// Compression algorithm used for the stub archive.
        #[clap(long, default_value_t = Compression::Zip)]
        stub_compression: Compression,
        /// Path to the submission archive.
        #[clap(long)]
        submission_path: PathBuf,
        /// Compression algorithm used for the submission.
        #[clap(long, default_value_t = Compression::Zip)]
        submission_compression: Compression,
        /// A key-value pair in the form key=value to be written into .tmcparams. If multiple pairs with the same key are given, the values are collected into an array.
        #[clap(long)]
        tmc_param: Vec<String>,
        /// If given, the contents in the resulting archive will be nested inside a directory with this name.
        #[clap(long)]
        top_level_dir_name: Option<String>,
    },

    /// Refresh the given course
    RefreshCourse {
        /// Path to the cached course.
        #[clap(long)]
        cache_path: PathBuf,
        /// The cache root.
        #[clap(long)]
        cache_root: PathBuf,
        /// The name of the course.
        #[clap(long)]
        course_name: String,
        /// Version control branch.
        #[clap(long)]
        git_branch: String,
        /// Version control URL or path.
        #[clap(long)]
        source_url: String,
    },

    /// Run the tests for the exercise using the appropriate language plugin
    #[clap(long_about = schema_leaked::<RunResult>())]
    RunTests {
        /// Runs checkstyle if given. Path to the file where the style results will be written.
        #[clap(long, requires = "locale")]
        checkstyle_output_path: Option<PathBuf>,
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'. Required if checkstyle-output-path is given.
        #[clap(long)]
        locale: Option<Locale>,
        /// If defined, the test results will be written to this path. Overwritten if it already exists.
        #[clap(long)]
        output_path: Option<PathBuf>,
        /// If defined, the command will wait for a string to be written to stdin, used for signing the output file with jwt.
        #[clap(long)]
        wait_for_secret: bool,
    },

    #[clap(subcommand)]
    Settings(Settings),

    /// Produces a description of an exercise using the appropriate language plugin
    #[clap(long_about = schema_leaked::<ExerciseDesc>())]
    ScanExercise {
        /// Path to the directory where the project resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// If given, the scan results will be written to this path. Overwritten if it already exists.
        #[clap(long)]
        output_path: Option<PathBuf>,
    },
}

/// Various commands that communicate with the TMC server
#[derive(Parser)]
#[clap(subcommand_required(true), arg_required_else_help(true))]
pub enum Core {
    /// Checks for updates to any exercises that exist locally.
    #[clap(long_about = schema_leaked::<Vec<UpdatedExercise>>())]
    CheckExerciseUpdates,

    /// Downloads an exercise's model solution
    #[clap(long_about = SCHEMA_NULL)]
    DownloadModelSolution {
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
        /// Path to where the model solution will be downloaded.
        #[clap(long)]
        target: PathBuf,
    },

    /// Downloads an old submission. Resets the exercise at output-path if any, downloading the exercise base from the server. The old submission is then downloaded and extracted on top of the base, using the student file policy to retain student files
    #[clap(long_about = SCHEMA_NULL)]
    DownloadOldSubmission {
        /// The ID of the submission.
        #[clap(long)]
        submission_id: u32,
        /// If set, the exercise is submitted to the server before resetting it.
        #[clap(long)]
        save_old_state: bool,
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
        /// Path to where the submission should be downloaded.
        #[clap(long)]
        output_path: PathBuf,
    },

    /// Downloads exercises. If downloading an exercise that has been downloaded before, the student file policy will be used to avoid overwriting student files, effectively just updating the exercise files
    #[clap(long_about = schema_leaked::<DownloadOrUpdateCourseExercisesResult>())]
    DownloadOrUpdateCourseExercises {
        /// If set, will always download the course template instead of the latest submission, even if one exists.
        #[clap(long)]
        download_template: bool,
        /// Exercise id of an exercise that should be downloaded. Multiple ids can be given.
        #[clap(long, num_args = 1..)]
        exercise_id: Vec<u32>,
    },

    ///Fetches course data. Combines course details, course exercises and course settings
    #[clap(long_about = schema_leaked::<CombinedCourseData>())]
    GetCourseData {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
    },

    /// Fetches course details
    #[clap(long_about = schema_leaked::<CourseDetails>())]
    GetCourseDetails {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
    },

    /// Lists a course's exercises
    #[clap(long_about = schema_leaked::<Vec<CourseExercise>>())]
    GetCourseExercises {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
    },

    /// Fetches course settings
    #[clap(long_about = schema_leaked::<CourseData>())]
    GetCourseSettings {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
    },

    /// Lists courses
    #[clap(long_about = schema_leaked::<Vec<CourseData>>())]
    GetCourses {
        /// Organization slug (e.g. mooc, hy).
        #[clap(long)]
        organization: String,
    },

    /// Fetches exercise details
    #[clap(long_about = schema_leaked::<ExerciseDetails>())]
    GetExerciseDetails {
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
    },

    /// Fetches the current user's old submissions for an exercise
    #[clap(long_about = schema_leaked::<Vec<Submission>>())]
    GetExerciseSubmissions {
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
    },

    /// Checks for updates to exercises
    #[clap(long_about = schema_leaked::<UpdateResult>())]
    GetExerciseUpdates {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
        /// An exercise. Takes two values, an exercise id and a checksum. Multiple exercises can be given.
        #[clap(long, required = true, number_of_values = 2, value_names = &["exercise-id", "checksum"])]
        exercise: Vec<String>,
    },

    /// Fetches an organization
    #[clap(long_about = schema_leaked::<Organization>())]
    GetOrganization {
        /// Organization slug (e.g. mooc, hy).
        #[clap(long)]
        organization: String,
    },

    /// Fetches a list of all organizations from the TMC server
    #[clap(long_about = schema_leaked::<Vec<Organization>>())]
    GetOrganizations,

    /// Fetches unread reviews for a course
    #[clap(long_about = schema_leaked::<Vec<Review>>())]
    GetUnreadReviews {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
    },

    /// Checks if the CLI is authenticated. Prints the access token if so
    #[clap(long_about = SCHEMA_TOKEN)]
    LoggedIn,

    /// Authenticates with the TMC server and stores the OAuth2 token in config. You can log in either by email and password or an access token
    #[clap(long_about = SCHEMA_NULL)]
    Login {
        /// If set, the password is expected to be a base64 encoded string. This can be useful if the password contains special characters.
        #[clap(long)]
        base64: bool,
        /// The email address of your TMC account. The password will be read through stdin.
        #[clap(long, required_unless_present = "set_access_token")]
        email: Option<String>,
        /// The OAUTH2 access token that should be used for authentication.
        #[clap(long, required_unless_present = "email")]
        set_access_token: Option<String>,
        /// If set, the password will be read from stdin instead of TTY like usual.
        /// The keyboard input is not hidden in this case, so this should only be used when running the CLI programmatically.
        #[clap(long)]
        stdin: bool,
    },

    /// Logs out and removes the OAuth2 token from config
    #[clap(long_about = SCHEMA_NULL)]
    Logout,

    /// Marks a review as read
    #[clap(long_about = SCHEMA_NULL)]
    MarkReviewAsRead {
        /// The ID of the course.
        #[clap(long)]
        course_id: u32,
        /// The ID of the review.
        #[clap(long)]
        review_id: u32,
    },

    /// Sends an exercise to the TMC pastebin
    #[clap(long_about = schema_leaked::<NewSubmission>())]
    Paste {
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
        /// Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.
        #[clap(long)]
        locale: Option<Locale>,
        /// Optional message to attach to the paste.
        #[clap(long)]
        paste_message: Option<String>,
        /// Path to the exercise to be submitted.
        #[clap(long)]
        submission_path: PathBuf,
    },

    /// Requests code review
    #[clap(long_about = schema_leaked::<NewSubmission>())]
    RequestCodeReview {
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
        /// Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.
        #[clap(long)]
        locale: Locale,
        /// Message for the review.
        #[clap(long)]
        message_for_reviewer: Option<String>,
        /// Path to the directory where the submission resides.
        #[clap(long)]
        submission_path: PathBuf,
    },

    /// Resets an exercise. Removes the contents of the exercise directory and redownloads it from the server
    #[clap(long_about = SCHEMA_NULL)]
    ResetExercise {
        /// If set, the exercise is submitted to the server before resetting it.
        #[clap(long)]
        save_old_state: bool,
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
        /// Path to the directory where the project resides.
        #[clap(long)]
        exercise_path: PathBuf,
    },

    /// Sends feedback for an exercise submission
    #[clap(long_about = schema_leaked::<SubmissionFeedbackResponse>())]
    SendFeedback {
        /// The ID of the submission.
        #[clap(long, required_unless_present = "feedback_url")]
        submission_id: Option<u32>,
        /// The feedback answer URL.
        #[clap(long, required_unless_present = "submission_id")]
        feedback_url: Option<String>,
        /// A feedback answer. Takes two values, a feedback answer id and the answer. Multiple feedback arguments can be given.
        #[clap(long, required = true, number_of_values = 2, value_names = &["feedback-answer-id, answer"])]
        feedback: Vec<String>,
    },

    /// Submits an exercise. By default blocks until the submission results are returned
    #[clap(long_about = schema_leaked::<SubmissionFinished>())]
    Submit {
        /// Set to avoid blocking.
        #[clap(long)]
        dont_block: bool,
        /// Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.
        #[clap(long)]
        locale: Option<Locale>,
        /// Path to the directory where the exercise resides.
        #[clap(long)]
        submission_path: PathBuf,
        /// The ID of the exercise.
        #[clap(long)]
        exercise_id: u32,
    },

    /// Updates all local exercises that have been updated on the server
    #[clap(long_about = SCHEMA_NULL)]
    UpdateExercises,

    /// Waits for a submission to finish
    #[clap(long_about = schema_leaked::<SubmissionFinished>())]
    WaitForSubmission {
        /// The ID of the submission.
        #[clap(long)]
        submission_id: u32,
    },
}

/// Configure the CLI
#[derive(Parser)]
#[clap(subcommand_required(true), arg_required_else_help(true))]
pub enum Settings {
    /// Retrieves a value from the settings
    Get {
        /// The name of the setting.
        setting: String,
    },
    /// Prints every key=value pair in the settings file
    List,
    /// Migrates an exercise on disk into the langs project directory
    Migrate {
        /// Path to the directory where the project resides.
        #[clap(long)]
        exercise_path: PathBuf,
        /// The course slug, e.g. mooc-java-programming-i.
        #[clap(long)]
        course_slug: String,
        /// The exercise id, e.g. 1234.
        #[clap(long)]
        exercise_id: u32,
        /// The exercise slug, e.g. part01-Part01_01.Sandbox.
        #[clap(long)]
        exercise_slug: String,
        /// The checksum of the exercise from the TMC server.
        #[clap(long)]
        exercise_checksum: String,
    },
    /// Change the projects-dir setting, moving the contents into the new directory
    MoveProjectsDir {
        /// The directory where the projects should be moved.
        dir: PathBuf,
    },
    /// Resets the settings file to the defaults
    Reset,
    /// Saves a value in the settings
    Set {
        /// The key. Parsed as JSON, assumed to be a string if parsing fails.
        key: String,
        /// The value in JSON.
        json: String,
        #[clap(long)]
        base64: bool,
    },
    /// Unsets a value from the settings
    Unset {
        /// The name of the setting.
        setting: String,
    },
}

#[derive(Clone, Copy)]
pub struct Locale(pub Language);

impl FromStr for Locale {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let locale = Language::from_locale(s)
            .or_else(|| Language::from_639_1(s))
            .or_else(|| Language::from_639_3(s))
            .with_context(|| format!("Invalid locale: {}", s))?;
        Ok(Locale(locale))
    }
}

// == utilities for printing the JSON schema of the objects printed to stdout by the CLI ==
const SCHEMA_NULL: &str = "Result data JSON format: null";
const SCHEMA_TOKEN: &str = r#"Result data JSON format:
{
    "access_token": String,
    "token_type": String,
    "scope": String,
}"#;

// clap's long_about only accepts string slices, so
// this function is used to leak a constant amount of
// memory to dynamically create static slices
// todo: stop leaking memory
fn schema_leaked<T: JsonSchema>() -> &'static str {
    let schema = schemars::schema_for!(T);
    let json = format!(
        "Result data JSON format:\n{}",
        serde_json::to_string_pretty(&schema).expect("serialization should not fail")
    );
    Box::leak(Box::new(json))
}

#[cfg(test)]
mod base_test {
    use super::*;

    fn get_matches(args: &[&str]) {
        Opt::parse_from(
            &[
                "tmc-langs-cli",
                "--client-name",
                "client",
                "--client-version",
                "version",
            ]
            .iter()
            .chain(args)
            .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn sanity() {
        assert!(
            Opt::try_parse_from(&["tmc-langs-cli", "checkstyle", "--non-existent-arg"]).is_err()
        );
    }

    #[test]
    fn checkstyle() {
        get_matches(&[
            "checkstyle",
            "--exercise-path",
            "path",
            "--locale",
            "fi",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn clean() {
        get_matches(&["clean", "--exercise-path", "path"]);
    }

    #[test]
    fn compress_project() {
        get_matches(&[
            "compress-project",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    /*
    #[test]
    fn disk_space() {
        get_matches(&["disk-space", "--path", "path"]);
    }
    */

    #[test]
    fn extract_project() {
        get_matches(&[
            "extract-project",
            "--archive-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn fast_available_points() {
        get_matches(&["fast-available-points", "--exercise-path", "path"]);
    }

    #[test]
    fn find_exercises() {
        get_matches(&[
            "find-exercises",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn get_exercise_packaging_configuration() {
        get_matches(&[
            "get-exercise-packaging-configuration",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn list_local_course_exercises() {
        get_matches(&["list-local-course-exercises", "--course-slug", "slug"]);
    }

    #[test]
    fn prepare_solutions() {
        get_matches(&[
            "prepare-solution",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn prepare_stubs() {
        get_matches(&[
            "prepare-stub",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn prepare_submission() {
        get_matches(&[
            "prepare-submission",
            "--clone-path",
            "path",
            "--output-format",
            "tar",
            "--output-path",
            "path",
            "--stub-zip-path",
            "path",
            "--submission-path",
            "path",
            "--tmc-param",
            "a=b",
            "--tmc-param",
            "c=d",
        ]);
    }

    #[test]
    fn refresh_course() {
        get_matches(&[
            "refresh-course",
            "--cache-path",
            "path",
            "--cache-root",
            "path",
            "--course-name",
            "name",
            "--git-branch",
            "main",
            "--source-url",
            "http://example.com",
        ]);
    }

    #[test]
    fn run_tests() {
        get_matches(&[
            "run-tests",
            "--checkstyle-output-path",
            "path",
            "--exercise-path",
            "path",
            "--locale",
            "fi",
            "--output-path",
            "path",
        ]);
    }

    #[test]
    fn scan_exercise() {
        get_matches(&[
            "scan-exercise",
            "--exercise-path",
            "path",
            "--output-path",
            "path",
        ]);
    }
}

#[cfg(test)]
mod core_test {
    use super::*;

    fn get_matches_core(args: &[&str]) {
        Opt::parse_from(
            &[
                "tmc-langs-cli",
                "--client-name",
                "client",
                "--client-version",
                "version",
                "core",
            ]
            .iter()
            .chain(args)
            .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn check_exercise_updates() {
        get_matches_core(&["check-exercise-updates"]);
    }

    #[test]
    fn download_model_solution() {
        get_matches_core(&[
            "download-model-solution",
            "--exercise-id",
            "0",
            "--target",
            "path",
        ]);
    }

    #[test]
    fn download_old_submission() {
        get_matches_core(&[
            "download-old-submission",
            "--save-old-state",
            "--exercise-id",
            "1234",
            "--output-path",
            "path",
            "--submission-id",
            "2345",
        ]);
    }

    #[test]
    fn download_or_update_course_exercises() {
        get_matches_core(&[
            "download-or-update-course-exercises",
            "--exercise-id",
            "1234",
            "--exercise-id",
            "2345",
        ]);
        get_matches_core(&[
            "download-or-update-course-exercises",
            "--exercise-id",
            "1234",
            "2345",
        ]);
    }

    #[test]
    fn get_course_data() {
        get_matches_core(&["get-course-data", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_details() {
        get_matches_core(&["get-course-details", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_exercises() {
        get_matches_core(&["get-course-exercises", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_settings() {
        get_matches_core(&["get-course-settings", "--course-id", "1234"]);
    }

    #[test]
    fn get_courses() {
        get_matches_core(&["get-courses", "--organization", "org"]);
    }

    #[test]
    fn get_exercise_details() {
        get_matches_core(&["get-exercise-details", "--exercise-id", "1234"]);
    }

    #[test]
    fn get_exercise_submissions() {
        get_matches_core(&["get-exercise-submissions", "--exercise-id", "1234"]);
    }

    #[test]
    fn get_exercise_updates() {
        get_matches_core(&[
            "get-exercise-updates",
            "--course-id",
            "1234",
            "--exercise",
            "1234",
            "abcd",
            "--exercise",
            "2345",
            "bcde",
        ]);
    }

    #[test]
    fn get_organization() {
        get_matches_core(&["get-organization", "--organization", "org"]);
    }

    #[test]
    fn get_organizations() {
        get_matches_core(&["get-organizations"]);
    }

    #[test]
    fn get_unread_reviews() {
        get_matches_core(&["get-unread-reviews", "--course-id", "0"]);
    }

    #[test]
    fn logged_in() {
        get_matches_core(&["logged-in"]);
    }

    #[test]
    fn login() {
        get_matches_core(&[
            "login",
            "--base64",
            "--email",
            "email",
            "--set-access-token",
            "access token",
        ]);
    }

    #[test]
    fn logout() {
        get_matches_core(&["logout"]);
    }

    #[test]
    fn mark_review_as_read() {
        get_matches_core(&[
            "mark-review-as-read",
            "--course-id",
            "0",
            "--review-id",
            "1",
        ]);
    }

    #[test]
    fn paste() {
        get_matches_core(&[
            "paste",
            "--locale",
            "fi",
            "--paste-message",
            "msg",
            "--submission-path",
            "path",
            "--exercise-id",
            "0",
        ]);
    }

    #[test]
    fn request_code_review() {
        get_matches_core(&[
            "request-code-review",
            "--locale",
            "fi",
            "--message-for-reviewer",
            "msg",
            "--submission-path",
            "path",
            "--exercise-id",
            "0",
        ]);
    }

    #[test]
    fn reset_exercise() {
        get_matches_core(&[
            "reset-exercise",
            "--save-old-state",
            "--exercise-id",
            "1234",
            "--exercise-path",
            "path",
        ]);
    }

    #[test]
    fn send_feedback() {
        get_matches_core(&[
            "send-feedback",
            "--feedback",
            "1234",
            "answer",
            "--submission-id",
            "0",
        ]);
    }

    #[test]
    fn submit() {
        get_matches_core(&[
            "submit",
            "--dont-block",
            "--locale",
            "fi",
            "--submission-path",
            "path",
            "--exercise-id",
            "0",
        ]);
    }

    #[test]
    fn update_exercises() {
        get_matches_core(&["update-exercises"]);
    }

    #[test]
    fn wait_for_submission() {
        get_matches_core(&["wait-for-submission", "--submission-id", "0"]);
    }
}

#[cfg(test)]
mod settings_test {
    use super::*;

    fn get_matches_settings(args: &[&str]) {
        Opt::parse_from(
            &["tmc-langs-cli", "--client-name", "client", "settings"]
                .iter()
                .chain(args)
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn get() {
        get_matches_settings(&["get", "key"]);
    }

    #[test]
    fn list() {
        get_matches_settings(&["list"]);
    }

    #[test]
    fn migrate() {
        get_matches_settings(&[
            "migrate",
            "--exercise-path",
            "path",
            "--course-slug",
            "slug",
            "--exercise-id",
            "1234",
            "--exercise-slug",
            "slug",
            "--exercise-checksum",
            "abcd",
        ]);
    }

    #[test]
    fn move_projects_dir() {
        get_matches_settings(&["move-projects-dir", "path"]);
    }

    #[test]
    fn reset() {
        get_matches_settings(&["reset"]);
    }

    #[test]
    fn set() {
        get_matches_settings(&["set", "key", "\"json\""]);
    }

    #[test]
    fn unset() {
        get_matches_settings(&["unset", "key"]);
    }
}
