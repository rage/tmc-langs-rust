// Python plugin error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::FileIo;

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON")]
    Deserialize(PathBuf, #[source] serde_json::Error),
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

    #[error("File IO error")]
    FileIo(#[from] FileIo),
    #[error("Error")]
    Tmc(#[from] tmc_langs_framework::TmcError),
}

impl From<PythonError> for TmcError {
    fn from(err: PythonError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

impl<T> Into<Result<T, TmcError>> for PythonError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
