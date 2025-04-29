//! Contains the type definition for the output format of the CLI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tmc_langs::{
    CombinedCourseData, ConfigValue, DownloadOrUpdateCourseExercisesResult, ExerciseDesc,
    ExerciseDownload, ExercisePackagingConfiguration, LocalExercise, RunResult,
    StyleValidationResult, TmcConfig, UpdatedExercise, mooc,
    notification_reporter::Notification,
    tmc::{
        ClientUpdateData, Token, UpdateResult,
        response::{
            Course, CourseData, CourseDetails, CourseExercise, ExerciseDetails, NewSubmission,
            Organization, Review, Submission, SubmissionFeedbackResponse, SubmissionFinished,
        },
    },
};
use tmc_langs_util::progress_reporter::StatusUpdate;

/// The format for all messages written to stdout by the CLI
#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct OutputData {
    pub status: Status,
    pub message: String,
    pub result: OutputResult,
    pub data: Option<DataKind>,
}

#[derive(Debug, Serialize)]
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
    LocalExercises(Vec<LocalExercise>),
    RefreshResult(tmc_langs::RefreshData),
    TestResult(RunResult),
    ExerciseDesc(ExerciseDesc),
    UpdatedExercises(Vec<UpdatedExercise>),
    ExerciseDownload(DownloadOrUpdateCourseExercisesResult),
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
    Token(#[cfg_attr(feature = "ts-rs", ts(type = "unknown"))] Token),
    NewSubmission(NewSubmission),
    SubmissionFeedbackResponse(SubmissionFeedbackResponse),
    SubmissionFinished(SubmissionFinished),
    ConfigValue(ConfigValue),
    TmcConfig(TmcConfig),
    CompressedProjectHash(String),
    SubmissionSandbox(String),
    MoocCourseInstances(Vec<mooc::CourseInstance>),
    MoocExerciseSlides(Vec<mooc::TmcExerciseSlide>),
    MoocExerciseSlide(mooc::TmcExerciseSlide),
    MoocSubmissionFinished(mooc::ExerciseTaskSubmissionResult),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "update-data-kind")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum StatusUpdateData {
    ClientUpdateData(StatusUpdate<ClientUpdateData>),
    None(StatusUpdate<()>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum Status {
    /// The command was ran without fatal errors
    Finished,
    /// An unexpected issue occurred during the command
    Crashed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum OutputResult {
    LoggedIn,
    LoggedOut,
    NotLoggedIn,
    Error,
    ExecutedCommand,
}

#[derive(Debug, Serialize, Deserialize)]
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
    /// Failed to download some or all exercises
    FailedExerciseDownload {
        completed: Vec<ExerciseDownload>,
        skipped: Vec<ExerciseDownload>,
        failed: Vec<(ExerciseDownload, Vec<String>)>,
    },
}

pub use tmc_langs::ProjectsDirExercise;

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadTarget {
    pub id: u32,
    pub path: PathBuf,
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
            data: Some(DataKind::ExerciseDownload(
                DownloadOrUpdateCourseExercisesResult {
                    downloaded: vec![
                        ExerciseDownload {
                            id: 1,
                            course_slug: "some course".to_string(),
                            exercise_slug: "some exercise".to_string(),
                            path: PathBuf::from("some path"),
                        },
                        ExerciseDownload {
                            id: 2,
                            course_slug: "some course".to_string(),
                            exercise_slug: "another exercise".to_string(),
                            path: PathBuf::from("another path"),
                        },
                    ],
                    skipped: vec![ExerciseDownload {
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
}
