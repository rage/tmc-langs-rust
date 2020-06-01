use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Error, Debug)]
pub enum JavaError {
    #[error("No java.home found in Java properties")]
    NoJavaHome,
    #[error("Maven did not output any class path")]
    NoMvnClassPath,
    #[error("Invalid exercise")]
    InvalidExercise,
    #[error("Failed to run {0}: {1}")]
    FailedToRun(&'static str, std::io::Error),
    #[error("Command '{0}' exited with an error, stderr: {}", String::from_utf8_lossy(&.1))]
    FailedCommand(&'static str, Vec<u8>),
    #[error("Failed to write temporary .jar files: {0}")]
    JarWrite(String),
    #[error("IO error with file {0}: {1}")]
    File(PathBuf, io::Error),
    #[error("IO error with directory {0}: {1}")]
    Dir(PathBuf, io::Error),
    #[error("IO error with temporary directory: {0}")]
    TempDir(io::Error),
    #[error("Failed to find home directory")]
    HomeDir,
    #[error("Failed to copy file from {0} to {1}: {2}")]
    FileCopy(PathBuf, PathBuf, std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Inner(#[from] TmcError),
}

impl From<JavaError> for TmcError {
    fn from(err: JavaError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

impl<T> Into<Result<T, TmcError>> for JavaError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
