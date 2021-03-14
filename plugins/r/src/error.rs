//! Error type for the R plugin

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::FileError;

#[derive(Debug, Error)]
pub enum RError {
    #[error("Failed to deserialize file {0} into JSON")]
    JsonDeserialize(PathBuf, #[source] serde_json::Error),

    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error("Error")]
    Tmc(#[from] TmcError),
}

// conversion from plugin error to TmcError::Plugin
impl From<RError> for TmcError {
    fn from(err: RError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> Into<Result<T, TmcError>> for RError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
