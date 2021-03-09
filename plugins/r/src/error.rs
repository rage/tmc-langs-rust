//! Error type for the R plugin

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::{
    error::{CommandError},
    TmcError,
};
use tmc_langs_util::FileIo;

#[derive(Debug, Error)]
pub enum RError {
    #[error("Failed to deserialize file {0} into JSON")]
    JsonDeserialize(PathBuf, #[source] serde_json::Error),

    #[error("Failed to run command")]
    Command(#[from] CommandError),
    #[error("File IO error")]
    FileIo(#[from] FileIo),
    #[error("Error")]
    Tmc(#[from] TmcError),
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
