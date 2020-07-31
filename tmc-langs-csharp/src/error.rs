//! Error type for C#
use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;
use tmc_langs_framework::{zip, TmcError};

#[derive(Debug, Error)]
pub enum CSharpError {
    #[error("Failed to create file {0}")]
    CreateFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file {0}")]
    WriteFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to read file {0}")]
    ReadFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove file {0}")]
    RemoveFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to create dir {0}")]
    CreateDir(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove dir {0}")]
    RemoveDir(PathBuf, #[source] std::io::Error),

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
    #[error("Failed to run {0}")]
    RunFailed(&'static str, #[source] std::io::Error),
    #[error("Command {0} failed with return code {1}")]
    CommandFailed(&'static str, ExitStatus),
}

impl From<CSharpError> for TmcError {
    fn from(err: CSharpError) -> Self {
        Self::Plugin(Box::new(err))
    }
}
