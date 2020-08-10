//! Output format

use serde::Serialize;
use tmc_langs_core::StatusType;

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
    Downloading,
    Compressing,
    Extracting,
    Processing,
    Sending,
    WaitingForResults,
    Finished,
    IntermediateStepFinished,
}

// converts a tmc_langs_core status to output result
impl From<StatusType> for OutputResult {
    fn from(status_type: StatusType) -> Self {
        match status_type {
            StatusType::Downloading => OutputResult::Downloading,
            StatusType::Compressing => OutputResult::Compressing,
            StatusType::Extracting => OutputResult::Extracting,
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
    AuthorizationError,
    /// Failed to connect to the TMC server, likely due to no internet connection
    ConnectionError,
}
