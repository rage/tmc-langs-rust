//! Contains the crate error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::error::FileIo;

#[derive(Debug, Error)]
pub enum UtilError {
    #[error("Failed to create temporary directory")]
    TempDir(#[source] std::io::Error),
    #[error("Invalid parameter key/value: {0}")]
    InvalidParam(String, #[source] ParamError),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip(PathBuf),
    #[error("Error appending path {0} to tar")]
    TarAppend(PathBuf, #[source] std::io::Error),

    #[error(transparent)]
    TmcError(#[from] tmc_langs_framework::TmcError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    FileIo(#[from] FileIo),
}

#[derive(Debug, Error)]
pub enum ParamError {
    #[error("Parameter key/value was empty")]
    Empty,
    #[error("Invalid character found in key/value: {0}")]
    InvalidChar(char),
}
