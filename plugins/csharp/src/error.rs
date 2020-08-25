//! Error type for C#
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::{error::CommandError, error::FileIo, zip, TmcError};

#[derive(Debug, Error)]
pub enum CSharpError {
    #[error("Failed to parse exercise description at {0}")]
    ParseExerciseDesc(PathBuf, #[source] serde_json::Error),
    #[error("Failed to parse test results at {0}")]
    ParseTestResults(PathBuf, #[source] serde_json::Error),

    #[error("Could not locate cache directory")]
    CacheDir,
    #[error("Could not locate boostrap DLL at {0}")]
    MissingBootstrapDll(PathBuf),
    #[error("Error while handling tmc-csharp-runner zip")]
    Zip(#[source] zip::result::ZipError),

    #[error("Command not found")]
    Command(#[from] CommandError),
    #[error("File IO error")]
    FileIo(#[from] FileIo),
    #[error("Error")]
    Tmc(#[from] TmcError),
}

impl From<CSharpError> for TmcError {
    fn from(err: CSharpError) -> Self {
        Self::Plugin(Box::new(err))
    }
}
