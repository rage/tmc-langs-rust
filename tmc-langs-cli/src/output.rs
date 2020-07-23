//! Output format

use serde::Serialize;

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
    Successful,
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
    Running,
    SentData,
    RetrievedData,
    ExecutedCommand,
}
