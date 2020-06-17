use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

use std::path::PathBuf;
use std::process::ExitStatus;

#[derive(Debug, Error)]
pub enum RError {
    #[error("Error running command {0}: {1}")]
    Command(&'static str, std::io::Error),

    #[error("Command {0} failed: {1}")]
    CommandStatus(&'static str, ExitStatus),

    #[error("IO error with file {0}: {1}")]
    Io(PathBuf, std::io::Error),

    #[error("JSON error with file {0}: {1}")]
    Json(PathBuf, serde_json::Error),
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
