//! Output format

use serde::Serialize;
use tmc_langs_core::StatusType;

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
    Finished,
    Crashed,
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
}

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
        }
    }
}
