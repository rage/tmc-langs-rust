//! Java error type

use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::{
    error::{CommandError, FileIo},
    TmcError,
};

#[derive(Error, Debug)]
pub enum JavaError {
    #[error("No java.home found in Java properties")]
    NoJavaHome,
    #[error("Maven did not output any class path")]
    NoMvnClassPath,
    #[error("{0} did not contain a valid exercise")]
    InvalidExercise(PathBuf),
    #[error("Failed to write temporary .jar file {0}")]
    JarWrite(PathBuf, #[source] io::Error),
    #[error("Failed to create temporary directory at")]
    TempDir(#[source] io::Error),
    #[error("Failed to find home directory")]
    HomeDir,
    #[error("Failed to find cache directory")]
    CacheDir,
    #[error("Failed to compile")]
    Compilation { stdout: String, stderr: String },

    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Failed to run command")]
    Command(#[from] CommandError),
    #[error("Error")]
    Tmc(#[from] TmcError),
}

impl From<JavaError> for TmcError {
    fn from(err: JavaError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

impl From<FileIo> for JavaError {
    fn from(err: FileIo) -> JavaError {
        JavaError::Tmc(TmcError::FileIo(err))
    }
}

impl<T> Into<Result<T, TmcError>> for JavaError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
