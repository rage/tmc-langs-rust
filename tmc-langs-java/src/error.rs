//! Java error type

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
    FailedToRun(String, #[source] std::io::Error),
    #[error(r"Command '{0}' exited with an error
#### STDOUT ####
{}
#### STDERR ####
{}", String::from_utf8_lossy(&.1), String::from_utf8_lossy(&.2))]
    FailedCommand(String, Vec<u8>, Vec<u8>),
    #[error("Failed to write temporary .jar files: {0}")]
    JarWrite(String),
    #[error("IO error with file {0}: {1}")]
    File(PathBuf, #[source] io::Error),
    #[error("IO error with directory {0}: {1}")]
    Dir(PathBuf, #[source] io::Error),
    #[error("IO error with temporary directory: {0}")]
    TempDir(#[source] io::Error),
    #[error("Failed to find home directory")]
    HomeDir,
    #[error("Failed to copy file from {0} to {1}: {2}")]
    FileCopy(PathBuf, PathBuf, #[source] std::io::Error),
    #[error("Failed to find cache directory")]
    CacheDir,
    #[error("Failed to unpack bundled mvn to {0}: {1}")]
    MvnUnpack(PathBuf, #[source] std::io::Error),

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
