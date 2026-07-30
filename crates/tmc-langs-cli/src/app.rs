//! Create clap app

use anyhow::Context;
use clap::Parser;
use schemars::JsonSchema;
use std::{path::PathBuf, str::FromStr};
use tmc_langs::{
    CombinedCourseData, Compression, DownloadOrUpdateMoocCourseExercisesResult,
    DownloadOrUpdateTmcCourseExercisesResult, ExerciseDesc, ExercisePackagingConfiguration,
    Language, LocalExercise, LocalMoocExercise, RunResult, StyleValidationResult, UpdatedExercise,
    mooc,
    tmc::{
        self, UpdateResult,
        response::{
            CourseData, CourseDetails, CourseExercise, ExerciseDetails, NewSubmission,
            Organization, Review, Submission, SubmissionFeedbackResponse, SubmissionFinished,
        },
    },
};
use uuid::Uuid;

#[derive(Parser)]
#[clap(
    name = env!("CARGO_PKG_NAME"),
    version,
    author,
    about,
    subcommand_required(true),
    arg_required_else_help(true)
)]
pub struct Cli {
    /// Pretty-prints all output
    #[clap(long, short)]
    pub pretty: bool,
    #[clap(subcommand)]
    pub command: Command,
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
        /// If set, does not include metadata such as timestamps in the archives.
        #[clap(long)]
        deterministic: bool,
        /// If set, simply compresses the target directory with all of its files.
        #[clap(long)]
        naive: bool,
    },

    /// Commands that communicate with the TMC server.
    Tmc(TestMyCode),

    /// Commands that communicate with the Mooc server.
    Mooc(Mooc),

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
        /// Path to the directory where to search for exercises.
        #[clap(long)]
        search_path: PathBuf,
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
    ListLocalTmcCourseExercises {
        /// The client name of which the exercises should be listed.
        #[clap(long)]
        client_name: String,
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

    /// Takes a submission archive and turns it into an archive with reset test files, and tmc-params, ready for further processing.
    /// Returns the sandbox image that should be used for the submission.
    #[clap(long_about = schema_leaked::<String>())]
    PrepareSubmission {
        /// The output format of the submission archive. Defaults to tar.
        #[clap(long, default_value_t = Compression::Tar)]
        output_format: Compression,
        /// Path to exercise's clone path, where the unmodified test files will be copied from.
        /// The course and exercise name will be derived from the clone path and used in the resulting submission archive,
        /// unless no-archive-prefix is set.
        #[clap(long)]
        clone_path: PathBuf,
        /// Path to the resulting archive. Overwritten if it already exists.
        #[clap(long)]
        output_path: PathBuf,
        /// If given, the tests will be copied from this stub instead, effectively ignoring hidden tests.
        #[clap(long)]
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
        /// If set, the submission is extracted without trying to find a project directory inside it. This can be useful if the submission is minimal and doesn't contain enough files to detect the project.
        #[clap(long)]
        extract_submission_naively: bool,
        /// A key-value pair in the form key=value to be written into .tmcparams. If multiple pairs with the same key are given, the values are collected into an array.
        #[clap(long)]
        tmc_param: Vec<String>,
        /// If given, the exercise will reside in the root of the archive.
        #[clap(long)]
        no_archive_prefix: bool,
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

    /// Prints the JSON Schema of the CLI's stdout output types to stdout.
    ///
    /// Prints the raw schema rather than a `CliOutput` envelope, matching the
    /// committed `bindings.schema.json` byte for byte so clients can check the
    /// binary against the schema they were built against.
    Schema,
}

/// Various commands that communicate with the TestMyCode server.
#[derive(Parser)]
#[clap(subcommand_required(true), arg_required_else_help(true))]
pub struct TestMyCode {
    /// Name used to differentiate between different frontends (e.g. the VSCode extension).
    #[clap(long, short)]
    pub client_name: String,
    /// Client version.
    #[clap(long, short = 'v')]
    pub client_version: String,
    #[clap(subcommand)]
    pub command: TestMyCodeCommand,
}

#[derive(Parser)]
pub enum TestMyCodeCommand {
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
    #[clap(long_about = schema_leaked::<DownloadOrUpdateTmcCourseExercisesResult>())]
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
    #[clap(long_about = schema_leaked::<Vec<tmc::response::Course>>())]
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

    /// Checks whether the CLI can authenticate with the TMC server, either with a
    /// stored TMC token or with the Courses MOOC access token. Prints the access
    /// token if so
    #[clap(long_about = SCHEMA_TOKEN)]
    LoggedIn,

    /// Removes a stored TMC OAuth2 token from config. Does not affect the Courses
    /// MOOC credentials; use `mooc logout` for those
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

#[derive(Parser, Clone)]
pub struct Mooc {
    /// Name used to differentiate between different frontends (e.g. the VSCode extension).
    #[clap(long, short)]
    pub client_name: String,
    #[clap(subcommand)]
    pub command: MoocCommand,
}

#[derive(Parser, Clone)]
pub enum MoocCommand {
    /// Logs in to the Courses MOOC backend using the OAuth2 device
    /// authorization grant (RFC 8628). Emits a `mooc-device-login` status
    /// update carrying the verification URL and user code, then blocks polling
    /// until the login is approved (or the parent process cancels by killing
    /// this one).
    #[clap(long_about = SCHEMA_NULL)]
    Login,
    /// Checks whether the CLI holds mooc credentials. Prints the access token if so.
    #[clap(long_about = SCHEMA_TOKEN)]
    LoggedIn,
    /// Logs out of the Courses MOOC backend, removing the stored credentials.
    #[clap(long_about = SCHEMA_NULL)]
    Logout,
    #[clap(long_about = schema_leaked::<Vec<Uuid>>())]
    CheckExerciseUpdates,
    /// Fetches information about a course.
    #[clap(long_about = schema_leaked::<mooc::Course>())]
    Course {
        #[clap(long)]
        course_id: Uuid,
    },
    /// Fetches the user's enrolled courses.
    #[clap(long_about = schema_leaked::<Vec<mooc::Course>>())]
    Courses,
    /// Fetches the available exercises for a course.
    CourseExercises {
        #[clap(long)]
        course_id: Uuid,
    },
    /// Fetches the current user's per-exercise progress for a course.
    #[clap(long_about = schema_leaked::<mooc::CourseProgress>())]
    CourseProgress {
        #[clap(long)]
        course_id: Uuid,
    },
    /// Fetches information about an exercise.
    Exercise {
        #[clap(long)]
        exercise_id: Uuid,
    },
    /// Downloads an exercise.
    DownloadExercise {
        #[clap(long)]
        exercise_id: Uuid,
        #[clap(long)]
        target: PathBuf,
    },
    /// Downloads or updates the given course exercises into the projects directory.
    #[clap(long_about = schema_leaked::<DownloadOrUpdateMoocCourseExercisesResult>())]
    DownloadOrUpdateCourseExercises {
        /// Exercise id of an exercise that should be downloaded. Multiple ids can be given.
        #[clap(long, num_args = 1..)]
        exercise_id: Vec<Uuid>,
        /// The course the exercises belong to. When given, only that course's
        /// slides are fetched to resolve the exercises instead of scanning every
        /// enrolled course.
        #[clap(long)]
        course_id: Option<Uuid>,
    },
    /// Lists the local exercises of a mooc course, looked up by course id.
    #[clap(long_about = schema_leaked::<Vec<LocalMoocExercise>>())]
    ListLocalCourseExercises {
        #[clap(long)]
        course_id: Uuid,
    },
    /// Submits an exercise, resolving the slide and editor task ids from the
    /// exercise id. By default blocks until grading reaches a terminal state,
    /// emitting progress updates.
    Submit {
        #[clap(long)]
        exercise_id: Uuid,
        #[clap(long)]
        submission_path: PathBuf,
        /// Return immediately with just the submission id instead of waiting for
        /// the grading to finish.
        #[clap(long)]
        dont_block: bool,
    },
    /// Waits for a submission's grading to reach a terminal state, polling the
    /// backend and emitting progress updates.
    WaitForGrading {
        #[clap(long)]
        submission_id: Uuid,
    },
    /// Submits an exercise (non-blocking) and shares the resulting submission,
    /// returning a public paste URL — the "share a submission" equivalent of TMC
    /// paste.
    #[clap(long_about = schema_leaked::<mooc::PasteResult>())]
    Paste {
        #[clap(long)]
        exercise_id: Uuid,
        #[clap(long)]
        submission_path: PathBuf,
    },
    /// Fetches the current user's past submissions to an exercise, newest first.
    #[clap(long_about = schema_leaked::<Vec<mooc::ExerciseSlideSubmissionListItem>>())]
    GetExerciseSubmissions {
        #[clap(long)]
        exercise_id: Uuid,
    },
    /// Downloads a past submission of an exercise, restoring its student files on
    /// top of a fresh exercise stub at `output_path`.
    DownloadOldSubmission {
        /// The id of the submission to download (an exercise-slide-submission id
        /// from `get-exercise-submissions`).
        #[clap(long)]
        submission_id: Uuid,
        /// The id of the exercise the submission belongs to.
        #[clap(long)]
        exercise_id: Uuid,
        /// Path to the directory where the project resides / should be restored.
        #[clap(long)]
        output_path: PathBuf,
        /// Submit the current state of `output_path` before overwriting it.
        #[clap(long)]
        save_old_state: bool,
    },
    /// Resets an exercise. Removes the contents of the exercise directory and
    /// re-downloads and re-extracts the exercise's stub archive from the server.
    #[clap(long_about = SCHEMA_NULL)]
    ResetExercise {
        /// If set, the exercise's current state is submitted to the server before resetting it.
        #[clap(long)]
        save_old_state: bool,
        /// The id of the exercise.
        #[clap(long)]
        exercise_id: Uuid,
        /// Path to the directory where the project resides.
        #[clap(long)]
        exercise_path: PathBuf,
    },
    /// Updates all local exercises that have been updated on the server
    #[clap(long_about = SCHEMA_NULL)]
    UpdateExercises,
}

/// Configure the CLI
#[derive(Parser)]
#[clap(subcommand_required(true), arg_required_else_help(true))]
pub struct Settings {
    /// The client name of which the settings should be listed.
    #[clap(long)]
    pub client_name: String,
    #[clap(subcommand)]
    pub command: SettingsCommand,
}

#[derive(Parser)]
pub enum SettingsCommand {
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
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Locale(#[cfg_attr(feature = "ts-rs", ts(type = "string"))] pub Language);

impl FromStr for Locale {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let locale = Language::from_locale(s)
            .or_else(|| Language::from_639_1(s))
            .or_else(|| Language::from_639_3(s))
            .with_context(|| format!("Invalid locale: {s}"))?;
        Ok(Self(locale))
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum CourseType {
    Tmc,
    Mooc,
}

impl FromStr for CourseType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ct = match s.to_lowercase().as_str() {
            "tmc" => Self::Tmc,
            "mooc" => Self::Mooc,
            other => return Err(anyhow::anyhow!("Invalid course type {other}")),
        };
        Ok(ct)
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
        Cli::try_parse_from(["tmc-langs-cli"].iter().chain(args).collect::<Vec<_>>()).unwrap();
    }

    #[test]
    fn sanity() {
        assert!(
            Cli::try_parse_from(["tmc-langs-cli", "checkstyle", "--non-existent-arg"]).is_err()
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
            "--search-path",
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
    fn list_local_tmc_course_exercises() {
        // The released spelling. Renaming a legacy command breaks every client
        // pinned to an older CLI, so this name is fixed.
        get_matches(&[
            "list-local-tmc-course-exercises",
            "--client-name",
            "client",
            "--course-slug",
            "slug",
        ]);
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
            "--stub-archive-path",
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

    fn get_matches_tmc(args: &[&str]) {
        Cli::try_parse_from(
            [
                "tmc-langs-cli",
                "tmc",
                "--client-name",
                "client",
                "--client-version",
                "version",
            ]
            .iter()
            .chain(args)
            .collect::<Vec<_>>(),
        )
        .unwrap();
    }

    #[test]
    fn check_exercise_updates() {
        get_matches_tmc(&["check-exercise-updates"]);
    }

    #[test]
    fn download_model_solution() {
        get_matches_tmc(&[
            "download-model-solution",
            "--exercise-id",
            "0",
            "--target",
            "path",
        ]);
    }

    #[test]
    fn download_old_submission() {
        get_matches_tmc(&[
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
        get_matches_tmc(&[
            "download-or-update-course-exercises",
            "--exercise-id",
            "1234",
            "--exercise-id",
            "2345",
        ]);
        get_matches_tmc(&[
            "download-or-update-course-exercises",
            "--exercise-id",
            "1234",
            "2345",
        ]);
    }

    #[test]
    fn get_course_data() {
        get_matches_tmc(&["get-course-data", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_details() {
        get_matches_tmc(&["get-course-details", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_exercises() {
        get_matches_tmc(&["get-course-exercises", "--course-id", "1234"]);
    }

    #[test]
    fn get_course_settings() {
        get_matches_tmc(&["get-course-settings", "--course-id", "1234"]);
    }

    #[test]
    fn get_courses() {
        get_matches_tmc(&["get-courses", "--organization", "org"]);
    }

    #[test]
    fn get_exercise_details() {
        get_matches_tmc(&["get-exercise-details", "--exercise-id", "1234"]);
    }

    #[test]
    fn get_exercise_submissions() {
        get_matches_tmc(&["get-exercise-submissions", "--exercise-id", "1234"]);
    }

    #[test]
    fn get_exercise_updates() {
        get_matches_tmc(&[
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
        get_matches_tmc(&["get-organization", "--organization", "org"]);
    }

    #[test]
    fn get_organizations() {
        get_matches_tmc(&["get-organizations"]);
    }

    #[test]
    fn get_unread_reviews() {
        get_matches_tmc(&["get-unread-reviews", "--course-id", "0"]);
    }

    #[test]
    fn logged_in() {
        get_matches_tmc(&["logged-in"]);
    }

    #[test]
    fn no_login_command() {
        // A new tmc username/password login no longer exists; a client
        // authenticates with a stored tmc token or the Courses MOOC one.
        Cli::try_parse_from([
            "tmc-langs-cli",
            "tmc",
            "--client-name",
            "client",
            "--client-version",
            "version",
            "login",
        ])
        .err()
        .expect("`tmc login` must not be accepted");
    }

    #[test]
    fn logout() {
        get_matches_tmc(&["logout"]);
    }

    #[test]
    fn mark_review_as_read() {
        get_matches_tmc(&[
            "mark-review-as-read",
            "--course-id",
            "0",
            "--review-id",
            "1",
        ]);
    }

    #[test]
    fn paste() {
        get_matches_tmc(&[
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
        get_matches_tmc(&[
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
        get_matches_tmc(&[
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
        get_matches_tmc(&[
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
        get_matches_tmc(&[
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
        get_matches_tmc(&["update-exercises"]);
    }

    #[test]
    fn wait_for_submission() {
        get_matches_tmc(&["wait-for-submission", "--submission-id", "0"]);
    }
}

#[cfg(test)]
mod settings_test {
    use super::*;

    fn get_matches_settings(args: &[&str]) {
        Cli::try_parse_from(
            ["tmc-langs-cli", "settings", "--client-name", "client"]
                .iter()
                .chain(args)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())
        .unwrap();
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

#[cfg(test)]
mod mooc_test {
    use super::*;

    fn get_matches_mooc(args: &[&str]) {
        Cli::try_parse_from(
            ["tmc-langs-cli", "mooc", "--client-name", "client"]
                .iter()
                .chain(args)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())
        .unwrap();
    }

    #[test]
    fn reset_exercise() {
        get_matches_mooc(&[
            "reset-exercise",
            "--save-old-state",
            "--exercise-id",
            "df5ee6c1-57d1-43b6-b39e-5d72119edb5f",
            "--exercise-path",
            "path",
        ]);
    }

    #[test]
    fn reset_exercise_without_save_old_state() {
        get_matches_mooc(&[
            "reset-exercise",
            "--exercise-id",
            "df5ee6c1-57d1-43b6-b39e-5d72119edb5f",
            "--exercise-path",
            "path",
        ]);
    }
}

#[cfg(test)]
mod test {
    use std::path::{Path, PathBuf};

    /// Path to the committed JSON Schema artifact.
    fn schema_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("bindings.schema.json")
    }

    /// Path to the committed TypeScript bindings artifact.
    #[cfg(feature = "ts-rs")]
    fn dts_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("bindings.d.ts")
    }

    #[test]
    #[ignore = "run manually to regenerate the committed bindings.schema.json"]
    fn generate_cli_bindings_schema() {
        std::fs::write(schema_path(), crate::output::cli_output_json_schema()).unwrap();
    }

    /// Drift gate: fails if `bindings.schema.json` no longer matches the
    /// current types.
    #[test]
    fn bindings_schema_up_to_date() {
        let committed = std::fs::read_to_string(schema_path()).expect(
            "bindings.schema.json should exist; regenerate it with \
             `cargo test -p tmc-langs-cli generate_cli_bindings_schema -- --ignored`",
        );
        let generated = crate::output::cli_output_json_schema();
        assert_eq!(
            committed, generated,
            "bindings.schema.json is out of date; regenerate it with \
             `cargo test -p tmc-langs-cli generate_cli_bindings_schema -- --ignored`"
        );
    }

    /// Drift gate: fails if `bindings.d.ts` no longer matches the current
    /// types. Requires `--features ts-rs`.
    #[test]
    #[cfg(feature = "ts-rs")]
    fn bindings_dts_up_to_date() {
        let committed = std::fs::read_to_string(dts_path()).expect(
            "bindings.d.ts should exist; regenerate it with \
             `cargo test -p tmc-langs-cli --features ts-rs generate_cli_bindings -- --ignored`",
        );
        let generated = generate_cli_bindings_dts();
        assert_eq!(
            committed, generated,
            "bindings.d.ts is out of date; regenerate it with \
             `cargo test -p tmc-langs-cli --features ts-rs generate_cli_bindings -- --ignored`"
        );
    }

    #[test]
    #[ignore = "run manually to regenerate the committed bindings.d.ts"]
    #[cfg(feature = "ts-rs")]
    fn generate_cli_bindings() {
        std::fs::write(dts_path(), generate_cli_bindings_dts()).unwrap();
    }

    /// Produces the TypeScript bindings as a string from the current types.
    #[cfg(feature = "ts-rs")]
    fn generate_cli_bindings_dts() -> String {
        let mut buf = Vec::new();
        ts_rs::export_to!(
            &mut buf,
            // input
            crate::app::Locale,
            // output
            crate::output::CliOutput,
            crate::output::DataKind,
            crate::output::Kind,
            crate::output::OutputData,
            crate::output::OutputResult,
            crate::output::Status,
            crate::output::StatusUpdateData,
            crate::output::MoocDeviceLogin,
            tmc_langs::notification_reporter::Notification,
            tmc_langs::notification_reporter::NotificationKind,
            tmc_langs::progress_reporter::StatusUpdate<()>,
            tmc_langs::tmc::ClientUpdateData,
            // checkstyle
            tmc_langs::StyleValidationResult,
            tmc_langs::StyleValidationError,
            tmc_langs::StyleValidationStrategy,
            // getExercisePackagingConfiguration
            tmc_langs::ExercisePackagingConfiguration,
            // listLocalCourseExercises
            tmc_langs::LocalExercise,
            tmc_langs::LocalTmcExercise,
            tmc_langs::LocalMoocExercise,
            // prepareSubmission
            tmc_langs::Compression,
            // refreshCourse
            tmc_langs::RefreshData,
            tmc_langs::RefreshExercise,
            tmc_langs::TmcProjectYml,
            tmc_langs::PythonVer,
            // runTests
            tmc_langs::RunResult,
            tmc_langs::RunStatus,
            tmc_langs::TestResult,
            // scanExercise
            tmc_langs::ExerciseDesc,
            tmc_langs::TestDesc,
            // checkExerciseUpdates
            tmc_langs::UpdatedExercise,
            // downloadOrUpdateCourseExercises
            tmc_langs::DownloadOrUpdateTmcCourseExercisesResult,
            tmc_langs::DownloadOrUpdateMoocCourseExercisesResult,
            tmc_langs::TmcExerciseDownload,
            tmc_langs::MoocExerciseDownload,
            // getCourseData
            tmc_langs::CombinedCourseData,
            // getCourseDetails
            tmc_langs::tmc::response::CourseDetails,
            tmc_langs::tmc::response::Exercise,
            tmc_langs::tmc::response::Course,
            // getCourseExercises
            tmc_langs::tmc::response::CourseExercise,
            tmc_langs::tmc::response::ExercisePoint,
            // getCourseSettings
            // getCourses
            tmc_langs::tmc::response::CourseData,
            // getExerciseDetails
            tmc_langs::tmc::response::ExerciseDetails,
            tmc_langs::tmc::response::ExerciseSubmission,
            // getExerciseSubmissions
            tmc_langs::tmc::response::Submission,
            // getExerciseUpdates
            tmc_langs::tmc::UpdateResult,
            // getOrganization
            // getOrganizations
            tmc_langs::tmc::response::Organization,
            // getUnreadReviews
            tmc_langs::tmc::response::Review,
            // paste
            // requestCodeReview
            tmc_langs::tmc::response::NewSubmission,
            // sendFeedback
            tmc_langs::tmc::response::SubmissionFeedbackResponse,
            tmc_langs::tmc::response::SubmissionStatus,
            // submit
            tmc_langs::tmc::response::TmcStyleValidationResult,
            tmc_langs::tmc::response::TmcStyleValidationError,
            tmc_langs::tmc::response::TmcStyleValidationStrategy,
            // waitForSubmission
            tmc_langs::tmc::response::SubmissionFinished,
            tmc_langs::tmc::response::TestCase,
            tmc_langs::tmc::response::SubmissionFeedbackQuestion,
            tmc_langs::tmc::response::SubmissionFeedbackKind,
            // settings get
            tmc_langs::ConfigValue,
            // settings list
            tmc_langs::TmcConfig,
            // mooc
            tmc_langs::mooc::Course,
            tmc_langs::mooc::TmcExerciseSlide,
            tmc_langs::mooc::TmcExerciseTask,
            tmc_langs::mooc::PublicSpec,
            tmc_langs::mooc::ModelSolutionSpec,
            tmc_langs::mooc::ExerciseFile,
            tmc_langs::mooc::ExerciseTaskSubmissionResult,
            tmc_langs::mooc::ExerciseTaskSubmissionStatus,
            tmc_langs::mooc::GradingProgress,
            tmc_langs::mooc::ExerciseSlideSubmissionListItem,
            tmc_langs::mooc::CourseProgress,
            tmc_langs::mooc::ExerciseProgress,
            tmc_langs::mooc::MoocClientUpdateData,
        )
        .unwrap();
        String::from_utf8(buf).unwrap()
    }
}
