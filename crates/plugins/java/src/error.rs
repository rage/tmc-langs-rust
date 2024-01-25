//! Java error type

use std::{io, path::PathBuf};
use thiserror::Error;
use tmc_langs_framework::TmcError;
use tmc_langs_util::{FileError, JsonError};

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
    #[error("Error while executing Java code")]
    Jvm { stdout: String, stderr: String },

    #[error("J4rs error
stdout: {}
stderr: {}
",
        stdout.as_deref().unwrap_or("none"),
        stderr.as_deref().unwrap_or("none")
    )]
    J4rs {
        stdout: Option<String>,
        stderr: Option<String>,
        source: j4rs::errors::J4RsError,
    },
    #[error("J4rs panicked: {0}")]
    J4rsPanic(String),
    #[error("This program does not support Java on this platform due to dynamic loading not being supported on musl. As a result, J4rs panicked: {0}")]
    UnsupportedPlatformMusl(String),

    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error("JSON error")]
    Json(#[from] JsonError),
    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error("Error")]
    Tmc(#[from] TmcError),
}

impl JavaError {
    pub fn j4rs(source: j4rs::errors::J4RsError) -> Self {
        Self::J4rs {
            stdout: None,
            stderr: None,
            source,
        }
    }
}

// conversion from plugin error to TmcError::Plugin
impl From<JavaError> for TmcError {
    fn from(err: JavaError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> From<JavaError> for Result<T, TmcError> {
    fn from(from: JavaError) -> Self {
        Err(TmcError::Plugin(Box::new(from)))
    }
}
