// Python plugin error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::{FileError, JsonError};

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON")]
    Deserialize(PathBuf, #[source] JsonError),
    #[error("Unexpected output when trying to print Python version: {0}")]
    VersionPrintError(String),
    #[error("Failed to parse Python version from {0}")]
    VersionParseError(String, #[source] std::num::ParseIntError),
    #[error("Python version found is too old: minimum major version required is {minimum_required}, but found {found}")]
    OldPythonVersion {
        found: String,
        minimum_required: String,
    },
    #[error("Failed to decode the test runner's HMAC as hex")]
    UnexpectedHmac,
    #[error("Failed to verify the test results")]
    InvalidHmac,
    #[error(
        "Failed to locate test results at {path}
    stdout: {stdout}
    stderr: {stderr}"
    )]
    MissingTestResults {
        path: PathBuf,
        stdout: String,
        stderr: String,
    },

    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error("Error")]
    Tmc(#[from] tmc_langs_framework::TmcError),
}

// conversion from plugin error to TmcError::Plugin
impl From<PythonError> for TmcError {
    fn from(err: PythonError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> From<PythonError> for Result<T, TmcError> {
    fn from(from: PythonError) -> Self {
        Err(TmcError::Plugin(Box::new(from)))
    }
}
