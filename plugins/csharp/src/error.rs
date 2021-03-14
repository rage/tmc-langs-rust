//! Error type for the crate.

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::FileError;

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
    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error("Zip error")]
    Zip(#[from] zip::result::ZipError),
}

// conversion from plugin error to TmcError::Plugin
impl From<CSharpError> for TmcError {
    fn from(err: CSharpError) -> Self {
        Self::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> Into<Result<T, TmcError>> for CSharpError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
