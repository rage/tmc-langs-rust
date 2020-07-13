// Python plugin error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("Error running command {0}: {1}")]
    Command(&'static str, #[source] std::io::Error),
    #[error("Path error for {0}: {1}")]
    Path(PathBuf, #[source] std::io::Error),
    #[error("Failed to open file {0}: {1}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to deserialize file at {0} to JSON: {1}")]
    Deserialize(PathBuf, #[source] serde_json::Error),
    #[error("Failed to remove file {0}: {1}")]
    FileRemove(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove directory {0}: {1}")]
    DirRemove(PathBuf, #[source] std::io::Error),
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
