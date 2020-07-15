//! Error type for the R plugin

use thiserror::Error;
use tmc_langs_framework::TmcError;

use std::path::PathBuf;
use std::process::ExitStatus;

#[derive(Debug, Error)]
pub enum RError {
    #[error("Error running command {0}")]
    Command(&'static str, #[source] std::io::Error),
    #[error("Command {0} failed with status {1}. stderr: {2}")]
    CommandStatus(&'static str, ExitStatus, String),

    #[error("Failed to open file {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove file {0}")]
    FileRemove(PathBuf, #[source] std::io::Error),

    #[error("Failed to deserialize file {0} into JSON")]
    JsonDeserialize(PathBuf, #[source] serde_json::Error),
}

impl From<RError> for TmcError {
    fn from(err: RError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

impl<T> Into<Result<T, TmcError>> for RError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
