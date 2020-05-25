use std::num::ParseIntError;
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Error, Debug)]
pub enum MakeError {
    #[error("No exercise found")]
    NoExerciseFound,
    #[error("Can't parse exercise description")]
    CantParseExerciseDesc,
    #[error("Failed to run tests without valgrind")]
    NoValgrindTests,
    #[error("Failed to run tests with valgrind")]
    ValgrindTests,
    #[error("Failed to parse valgrind logs")]
    ValgrindParse,
    #[error("Make finished unsuccessfully")]
    MakeFailed,

    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, std::io::Error),
    #[error("Failed to read file at {0}")]
    FileRead(PathBuf, std::io::Error),
    #[error("Failed to run make")]
    MakeCommand(std::io::Error),
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),
}

impl From<MakeError> for TmcError {
    fn from(other: MakeError) -> TmcError {
        TmcError::Plugin(Box::new(other))
    }
}

impl<T> Into<Result<T, TmcError>> for MakeError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
