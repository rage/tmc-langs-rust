//! Output format

use schemars::JsonSchema;
use serde::Serialize;
use std::path::PathBuf;
use tmc_langs_core::{CourseData, CourseDetails, CourseExercise};
use tmc_langs_util::progress_reporter::StatusUpdate;

/// The format for all messages written to stdout by the CLI
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "output-kind")]
pub enum Output<T: Serialize> {
    OutputData(OutputData<T>),
    StatusUpdate(StatusUpdate<T>),
    Warnings(Warnings),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct OutputData<T: Serialize> {
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
    /// Client out of date
    ObsoleteClient,
    /// Invalid token
    InvalidToken,
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
pub struct Warnings {
    warnings: Vec<String>,
}

impl Warnings {
    pub fn from_error_list(warnings: &[anyhow::Error]) -> Self {
        Self {
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
        }
    }
}
