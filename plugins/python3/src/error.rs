// Python plugin error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("Failed to open file {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON")]
    Deserialize(PathBuf, #[source] serde_json::Error),
    #[error("Failed to remove file {0}")]
    FileRemove(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove directory {0}")]
    DirRemove(PathBuf, #[source] std::io::Error),
    #[error(transparent)]
    Framework(#[from] tmc_langs_framework::TmcError),
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
