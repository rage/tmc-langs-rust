//! Output format

use schemars::JsonSchema;
use serde::Serialize;
use std::path::PathBuf;
use tmc_langs_core::{CourseData, CourseDetails, CourseExercise, StatusType};

/// The format for all messages written to stdout by the CLI
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Output<T: Serialize> {
    pub status: Status,
    pub message: Option<String>,
    pub result: OutputResult,
    pub percent_done: f64,
    pub data: Option<T>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// The command was ran without fatal errors
    Finished,
    /// An unexpected issue occurred during the command
    Crashed,
    /// The command is still in progress
    InProgress,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputResult {
    LoggedIn,
    LoggedOut,
    NotLoggedIn,
    Error,
    SentData,
    RetrievedData,
    ExecutedCommand,
    DownloadingExercise,
    DownloadedExercise,
    Processing,
    Sending,
    WaitingForResults,
    Finished,
    IntermediateStepFinished,
    PostedSubmission,
}

// converts a tmc_langs_core status to output result
impl From<StatusType> for OutputResult {
    fn from(status_type: StatusType) -> Self {
        match status_type {
            StatusType::DownloadingExercise { .. } => OutputResult::DownloadingExercise,
            StatusType::DownloadedExercise { .. } => OutputResult::DownloadedExercise,
            StatusType::PostedSubmission { .. } => OutputResult::PostedSubmission,
            StatusType::Processing => OutputResult::Processing,
            StatusType::Sending => OutputResult::Sending,
            StatusType::WaitingForResults => OutputResult::WaitingForResults,
            StatusType::Finished => OutputResult::Finished,
            StatusType::IntermediateStepFinished => OutputResult::IntermediateStepFinished,
        }
    }
}

/// The format for all error messages printed in Output.data
#[derive(Debug, Serialize)]
pub struct ErrorData {
    pub kind: Kind,
    /// Contains the error cause chain
    pub trace: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// For all other errors
    Generic,
    /// 403 from server
    Forbidden,
    /// Not logged in, detected either by no token or 401 from server
    NotLoggedIn,
    /// Failed to connect to the TMC server, likely due to no internet connection
    ConnectionError,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CombinedCourseData {
    pub details: CourseDetails,
    pub exercises: Vec<CourseExercise>,
    pub settings: CourseData,
}

#[derive(Debug, Serialize)]
pub struct DownloadTarget {
    pub id: usize,
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct SubmissionUrl {
    pub url: String,
}
