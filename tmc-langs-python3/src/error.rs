// Python plugin error type

use serde_json::Error as JsonError;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Error running command {0}: {1}")]
    Command(&'static str, std::io::Error),
    #[error("Path error for {0}: {1}")]
    Path(PathBuf, std::io::Error),
    #[error("Failed to open file {0}: {1}")]
    FileOpen(PathBuf, std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON: {1}")]
    Deserialize(PathBuf, JsonError),
    #[error("Failed to remove file {0}: {1}")]
    FileRemove(PathBuf, std::io::Error),
    #[error("Failed to remove directory {0}: {1}")]
    DirRemove(PathBuf, std::io::Error),
    #[error(transparent)]
    Framework(#[from] tmc_langs_framework::Error),
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
