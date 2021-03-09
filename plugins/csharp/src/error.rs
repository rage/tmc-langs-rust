//! Error type for the crate.

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::{CommandError, TmcError};
use tmc_langs_util::FileIo;

#[derive(Debug, Error)]
pub enum CSharpError {
    // Original error types.
    #[error("Failed to parse exercise description at {0}")]
    ParseExerciseDesc(PathBuf, #[source] serde_json::Error),
    #[error("Failed to parse test results at {0}")]
    ParseTestResults(PathBuf, #[source] serde_json::Error),
    #[error("Could not locate cache directory")]
    CacheDir,
    #[error("Could not locate boostrap DLL at {0}")]
    MissingBootstrapDll(PathBuf),

    // Wrapping other error types.
    #[error("Command not found")]
    Command(#[from] CommandError),
    #[error("File IO error")]
    FileIo(#[from] FileIo),
    #[error("TMC error")]
    Tmc(#[from] TmcError),
    #[error("Zip error")]
    Zip(#[from] zip::result::ZipError),
}

// conversion from plugin error to TmcError::Plugin
impl From<CSharpError> for TmcError {
    fn from(err: CSharpError) -> Self {
        Self::Plugin(Box::new(err))
    }
}
