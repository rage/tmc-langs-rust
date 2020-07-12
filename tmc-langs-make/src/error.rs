//! Error type for the make plugin.

use std::num::ParseIntError;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Error, Debug)]
pub enum MakeError {
    #[error("No exercise found")]
    NoExerciseFound,
    #[error("Can't parse exercise description: could not find tmc_available_points.txt")]
    CantFindAvailablePoints,
    #[error("Could not find tmc_available_points.xml")]
    CantFindTestResults,
    #[error("Failed to run tests without valgrind")]
    RunningTests,
    #[error("Failed to run tests with valgrind")]
    RunningTestsWithValgrind,
    #[error("Failed to parse valgrind logs")]
    ValgrindParse,
    #[error("Make finished unsuccessfully")]
    MakeFailed,

    #[error("Failed to parse XML at {0}: {1}")]
    XmlParseError(PathBuf, #[source] serde_xml_rs::Error),
    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read file at {0}")]
    FileRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to run make")]
    MakeCommand(#[source] std::io::Error),
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),
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
