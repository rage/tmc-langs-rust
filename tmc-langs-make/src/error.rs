//! Error type for the make plugin.

use std::num::ParseIntError;
use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;
use tmc_langs_framework::TmcError;

#[derive(Error, Debug)]
pub enum MakeError {
    #[error("No exercise found at {0}")]
    NoExerciseFound(PathBuf),
    #[error("Can't parse exercise description: could not find {0}")]
    CantFindAvailablePoints(PathBuf),
    #[error("Failed to run tests without valgrind. Exit code: {0}, stderr: {1}")]
    RunningTests(ExitStatus, String),
    #[error("Failed to run tests with valgrind. Exit code: {0}, stderr: {1}")]
    RunningTestsWithValgrind(ExitStatus, String),
    #[error("Failed to parse valgrind logs: could not find pids")]
    NoPidsInValgrindLogs,

    #[error("Failed to parse XML at {0}")]
    XmlParseError(PathBuf, #[source] serde_xml_rs::Error),
    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read file at {0}")]
    FileRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to run make")]
    MakeCommand(#[source] std::io::Error),
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),

    #[error(transparent)]
    TmcError(#[from] TmcError),
}

impl From<MakeError> for TmcError {
    fn from(other: MakeError) -> TmcError {
        TmcError::Plugin(Box::new(other))
    }
}

impl<T> Into<Result<T, TmcError>> for MakeError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
