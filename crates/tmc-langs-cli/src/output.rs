//! Contains the type definition for the output format of the CLI.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tmc_langs::{
    CombinedCourseData, ConfigValue, DownloadOrUpdateMoocCourseExercisesResult,
    DownloadOrUpdateTmcCourseExercisesResult, ExerciseDesc, ExercisePackagingConfiguration,
    LocalMoocExercise, LocalTmcExercise, RunResult, StyleValidationResult, TmcConfig,
    UpdatedExercise, mooc,
    notification_reporter::Notification,
    tmc::{
        ClientUpdateData, Token, UpdateResult,
        response::{
            Course, CourseData, CourseDetails, CourseExercise, ExerciseDetails, NewSubmission,
            Organization, Review, Submission, SubmissionFeedbackResponse, SubmissionFinished,
        },
    },
};
#[cfg(test)]
use tmc_langs::TmcExerciseDownload;
use tmc_langs_util::progress_reporter::StatusUpdate;
use uuid::Uuid;

/// The format for all messages written to stdout by the CLI
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "output-kind")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum CliOutput {
    /// Data that is output at the end of a command.
    OutputData(Box<OutputData>),
    /// Status update output as a command progresses.
    StatusUpdate(StatusUpdateData),
    /// Additional warnings, such as for an outdated Python dependency.
    Notification(Notification),
}

impl CliOutput {
    pub fn finished_with_data(message: impl Into<String>, data: DataKind) -> Self {
        Self::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: message.into(),
            result: OutputResult::ExecutedCommand,
            data: data.into(),
        }))
    }

    pub fn finished(message: impl Into<String>) -> Self {
        Self::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: message.into(),
            result: OutputResult::ExecutedCommand,
            data: None,
        }))
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct OutputData {
    pub status: Status,
    pub message: String,
    pub result: OutputResult,
    pub data: Option<DataKind>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "output-data-kind", content = "output-data")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum DataKind {
    Error {
        kind: Kind,
        trace: Vec<String>,
    },
    Validation(Option<StyleValidationResult>),
    /// megabytes
    // FreeDiskSpace(u64),
    AvailablePoints(Vec<String>),
    Exercises(Vec<PathBuf>),
    ExercisePackagingConfiguration(ExercisePackagingConfiguration),
    RefreshResult(tmc_langs::RefreshData),
    TestResult(RunResult),
    ExerciseDesc(ExerciseDesc),
    UpdatedExercises(Vec<UpdatedExercise>),
    MoocExerciseDownload(DownloadOrUpdateMoocCourseExercisesResult),
    CombinedCourseData(Box<CombinedCourseData>),
    CourseDetails(CourseDetails),
    CourseExercises(Vec<CourseExercise>),
    CourseData(CourseData),
    Courses(Vec<Course>),
    ExerciseDetails(ExerciseDetails),
    Submissions(Vec<Submission>),
    UpdateResult(UpdateResult),
    Organization(Organization),
    Organizations(Vec<Organization>),
    Reviews(Vec<Review>),
    Token(
        #[cfg_attr(feature = "ts-rs", ts(type = "unknown"))]
        #[schemars(with = "serde_json::Value")]
        Token,
    ),
    NewSubmission(NewSubmission),
    SubmissionFeedbackResponse(SubmissionFeedbackResponse),
    SubmissionFinished(SubmissionFinished),
    ConfigValue(ConfigValue),
    CompressedProjectHash(String),
    SubmissionSandbox(String),

    // tmc
    LocalTmcExercises(Vec<LocalTmcExercise>),
    TmcExerciseDownload(DownloadOrUpdateTmcCourseExercisesResult),
    TmcConfig(TmcConfig),

    // mooc
    MoocUpdatedExercises(Vec<Uuid>),
    LocalMoocExercises(Vec<LocalMoocExercise>),
    MoocCourse(mooc::Course),
    MoocCourses(Vec<mooc::Course>),
    MoocExerciseSlides(Vec<mooc::TmcExerciseSlide>),
    MoocExerciseSlide(mooc::TmcExerciseSlide),
    MoocSubmissionFinished(mooc::ExerciseTaskSubmissionResult),
    MoocSubmissionStatus(mooc::ExerciseTaskSubmissionStatus),
    MoocSubmissions(Vec<mooc::ExerciseSlideSubmissionListItem>),
    MoocPaste(mooc::PasteResult),
    MoocCourseProgress(mooc::CourseProgress),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "update-data-kind")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum StatusUpdateData {
    ClientUpdateData(StatusUpdate<ClientUpdateData>),
    /// Mooc's per-exercise download progress, mirroring `ClientUpdateData` for
    /// mooc's UUID-keyed exercises.
    MoocClientUpdateData(StatusUpdate<mooc::MoocClientUpdateData>),
    /// Emitted once at the start of `mooc login`, before the CLI blocks polling:
    /// carries the verification URL and user code the client shows the user to
    /// complete the OAuth2 device authorization login.
    MoocDeviceLogin(StatusUpdate<MoocDeviceLogin>),
    None(StatusUpdate<()>),
}

/// The data attached to a `mooc-device-login` status update. Mirrors the
/// relevant fields of the RFC 8628 device authorization response.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct MoocDeviceLogin {
    /// URL the user opens to enter the `user_code`.
    pub verification_uri: String,
    /// URL that already includes the `user_code`, if the server provided one.
    pub verification_uri_complete: Option<String>,
    /// The code the user enters (or confirms) on the verification page.
    pub user_code: String,
    /// Seconds until the device/user codes expire.
    pub expires_in: u32,
    /// Minimum seconds between token-endpoint polls.
    pub interval: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum Status {
    /// The command was ran without fatal errors
    Finished,
    /// An unexpected issue occurred during the command
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum OutputResult {
    LoggedIn,
    LoggedOut,
    NotLoggedIn,
    Error,
    ExecutedCommand,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum Kind {
    /// For all other errors
    Generic,
    /// 403 from server
    Forbidden,
    /// Not logged in, detected either by no token or 401 from server
    NotLoggedIn,
    /// Failed to connect to the TMC server, likely due to no internet connection
    ConnectionError,
    /// Client out of date
    ObsoleteClient,
    /// Invalid token
    InvalidToken,
    /// The user is not enrolled on the course this exercise belongs to
    /// (backend `message_key: "not_enrolled"`, HTTP 422)
    NotEnrolled,
}

pub use tmc_langs::ProjectsDirTmcExercise;

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadTarget {
    pub id: u32,
    pub path: PathBuf,
}

/// JSON Schema for everything the CLI writes to stdout, rooted at [`CliOutput`].
/// The single source of truth clients (e.g. tmc-vscode) validate against.
pub fn cli_output_schema() -> schemars::Schema {
    // Serialize contract, not deserialize: `#[serde(from = ...)]` types (e.g.
    // `CourseDetails`) deserialize through a wrapper but serialize flattened,
    // and `Option` omitted-vs-null differs between the two. `for_serialize()`
    // picks the wire format clients actually see.
    let settings = schemars::generate::SchemaSettings::draft2020_12().for_serialize();
    schemars::SchemaGenerator::new(settings).into_root_schema_for::<CliOutput>()
}

/// Returns [`cli_output_schema`] as pretty-printed JSON, terminated by a
/// newline — the exact bytes of the committed `bindings.schema.json` and of
/// the `tmc-langs-cli schema` subcommand's stdout.
pub fn cli_output_json_schema() -> String {
    let mut json = serde_json::to_string_pretty(&cli_output_schema())
        .expect("serializing a JSON schema should never fail");
    json.push('\n');
    json
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn read_api_file(filename: &str) -> String {
        std::fs::read_to_string(std::path::Path::new("api").join(filename)).unwrap()
    }

    #[test]
    fn output_data_none() {
        let output_data = CliOutput::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: "output with no data".to_string(),
            result: OutputResult::ExecutedCommand,
            data: None,
        }));
        let actual = serde_json::to_string_pretty(&output_data).unwrap();
        let expected = read_api_file("output-data-none.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn output_data_error() {
        let output_data = CliOutput::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: "errored!".to_string(),
            result: OutputResult::Error,
            data: Some(DataKind::Error {
                kind: Kind::Generic,
                trace: vec!["trace 1".to_string(), "trace 2".to_string()],
            }),
        }));
        let actual = serde_json::to_string_pretty(&output_data).unwrap();
        let expected = read_api_file("output-data-error.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn output_data_dl() {
        let output_data = CliOutput::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: "downloaded things".to_string(),
            result: OutputResult::ExecutedCommand,
            data: Some(DataKind::TmcExerciseDownload(
                DownloadOrUpdateTmcCourseExercisesResult {
                    downloaded: vec![
                        TmcExerciseDownload {
                            id: 1,
                            course_slug: "some course".to_string(),
                            exercise_slug: "some exercise".to_string(),
                            path: PathBuf::from("some path"),
                        },
                        TmcExerciseDownload {
                            id: 2,
                            course_slug: "some course".to_string(),
                            exercise_slug: "another exercise".to_string(),
                            path: PathBuf::from("another path"),
                        },
                    ],
                    skipped: vec![TmcExerciseDownload {
                        id: 3,
                        course_slug: "another course".to_string(),
                        exercise_slug: "some skipped exercise".to_string(),
                        path: PathBuf::from("third path"),
                    }],
                    failed: None,
                },
            )),
        }));
        let actual = serde_json::to_string_pretty(&output_data).unwrap();
        let expected = read_api_file("output-data-download-or-update.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn status_update() {
        let status_update =
            CliOutput::StatusUpdate(StatusUpdateData::ClientUpdateData(StatusUpdate {
                data: Some(ClientUpdateData::ExerciseDownload {
                    id: 1234,
                    path: PathBuf::from("some path"),
                }),
                finished: false,
                message: "doing things...".to_string(),
                percent_done: 33.3333,
                time: 2000,
            }));
        let actual = serde_json::to_string_pretty(&status_update).unwrap();
        let expected = read_api_file("status-update.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn notification() {
        let status_update = CliOutput::Notification(Notification::warning("some warning"));
        let actual = serde_json::to_string_pretty(&status_update).unwrap();
        let expected = read_api_file("warnings.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn mooc_client_update_data_status_update_shape() {
        // Locks the wire shape the VSCode side parses for mooc download progress:
        // outer `update-data-kind` = `mooc-client-update-data`, inner
        // `client-update-data-kind` = `exercise-download` with a UUID `id`.
        let id = Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap();
        let status_update =
            CliOutput::StatusUpdate(StatusUpdateData::MoocClientUpdateData(StatusUpdate {
                data: Some(mooc::MoocClientUpdateData::ExerciseDownload {
                    id,
                    path: PathBuf::from("some/path"),
                }),
                finished: false,
                message: "downloading...".to_string(),
                percent_done: 50.0,
                time: 1000,
            }));
        let actual = serde_json::to_value(&status_update).unwrap();
        assert_eq!(actual["output-kind"], "status-update");
        assert_eq!(actual["update-data-kind"], "mooc-client-update-data");
        assert_eq!(actual["data"]["client-update-data-kind"], "exercise-download");
        assert_eq!(
            actual["data"]["id"],
            "df5ee6c1-57d1-43b6-b39e-5d72119edb5f"
        );
        assert_eq!(actual["data"]["path"], "some/path");
    }
}
