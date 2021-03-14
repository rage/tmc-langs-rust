//! Java error type

use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::FileError;

#[derive(Error, Debug)]
pub enum JavaError {
    #[error("Path {0} was not valid UTF-8")]
    InvalidUtf8Path(PathBuf),
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

    #[error("J4RS error")]
    J4rs(#[from] j4rs::errors::J4RsError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error("JSON error")]
    Json(#[from] serde_json::Error),
    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error("Error")]
    Tmc(#[from] TmcError),
}

// conversion from plugin error to TmcError::Plugin
impl From<JavaError> for TmcError {
    fn from(err: JavaError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> Into<Result<T, TmcError>> for JavaError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
