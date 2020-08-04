//! Java error type

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;
use tmc_langs_framework::TmcError;

#[derive(Error, Debug)]
pub enum JavaError {
    #[error("No java.home found in Java properties")]
    NoJavaHome,
    #[error("Maven did not output any class path")]
    NoMvnClassPath,
    #[error("{0} did not contain a valid exercise")]
    InvalidExercise(PathBuf),
    #[error("Failed to run {0}")]
    FailedToRun(String, #[source] std::io::Error),
    #[error(r"Command '{0}' exited with a exit status {1}
#### STDOUT ####
{}
#### STDERR ####
{}", String::from_utf8_lossy(&.2), String::from_utf8_lossy(&.3))]
    FailedCommand(String, ExitStatus, Vec<u8>, Vec<u8>),
    #[error("Failed to write temporary .jar file {0}")]
    JarWrite(PathBuf, #[source] io::Error),
    #[error("Failed to create file at {0}")]
    FileCreate(PathBuf, #[source] io::Error),
    #[error("Failed to write to file at {0}")]
    FileWrite(PathBuf, #[source] io::Error),
    #[error("Failed to read to file at {0}")]
    FileRead(PathBuf, #[source] io::Error),
    #[error("Failed to remove file at {0}")]
    FileRemove(PathBuf, #[source] io::Error),
    #[error("Failed to create directory at {0}")]
    DirCreate(PathBuf, #[source] io::Error),
    #[error("Failed to create temporary directory at")]
    TempDir(#[source] io::Error),
    #[error("Failed to find home directory")]
    HomeDir,
    #[error("Failed to copy file from {0} to {1}")]
    FileCopy(PathBuf, PathBuf, #[source] std::io::Error),
    #[error("Failed to find cache directory")]
    CacheDir,
    #[error("Failed to compile")]
    Compilation(Vec<u8>),

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
