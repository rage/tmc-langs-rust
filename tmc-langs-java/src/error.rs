use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Error, Debug)]
pub enum JavaPluginError {
    #[error("No java.home found in Java properties")]
    NoJavaHome,
    #[error("Maven did not output any class path")]
    NoMvnClassPath,
    #[error("Invalid exercise")]
    InvalidExercise,
    #[error("Failed to run {0}")]
    FailedCommand(&'static str),
    #[error("Failed to write temporary .jar files: {0}")]
    JarWrite(String),
    #[error("IO error with file {0}: {1}")]
    File(PathBuf, io::Error),
    #[error("IO error with directory {0}: {1}")]
    Dir(PathBuf, io::Error),
    #[error("Failed to find home directory")]
    HomeDir,
}

impl From<JavaPluginError> for TmcError {
    fn from(err: JavaPluginError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

impl<T> Into<Result<T, TmcError>> for JavaPluginError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
