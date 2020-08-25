// Python plugin error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::{error::FileIo, TmcError};

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON")]
    Deserialize(PathBuf, #[source] serde_json::Error),

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
