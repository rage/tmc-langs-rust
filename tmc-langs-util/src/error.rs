//! Contains the crate error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::error::FileIo;

#[cfg(unix)]
use crate::task_executor::ModeBits;

#[derive(Debug, Error)]
pub enum UtilError {
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to create temporary directory")]
    TempDir(#[source] std::io::Error),
    #[error("Invalid parameter key/value: {0}")]
    InvalidParam(String, #[source] ParamError),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip(PathBuf),
    #[error("Failed to aquire mutex")]
    MutexError,
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),

    #[error("Error appending path {0} to tar")]
    TarAppend(PathBuf, #[source] std::io::Error),
    #[error("Error finishing tar")]
    TarFinish(#[source] std::io::Error),
    #[error("Error retrieving file handle from tar builder")]
    TarIntoInner(#[source] std::io::Error),
    #[error("Error compressing file at {0} with zstd")]
    Zstd(PathBuf, #[source] std::io::Error),
    #[error("Error while writing file to zip")]
    ZipWrite(#[source] std::io::Error),

    #[error("Cache path {0} was invalid. Not a valid UTF-8 string or did not contain a cache version after a dash")]
    InvalidCachePath(PathBuf),
    #[error("Path {0} contained a dash '-' which is currently not allowed")]
    InvalidDirectory(PathBuf),

    #[error(transparent)]
    TmcError(#[from] tmc_langs_framework::TmcError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    FileIo(#[from] FileIo),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Heim(#[from] heim::Error),

    #[cfg(unix)]
    #[error("Error changing permissions of {0}")]
    NixPermissionChange(PathBuf, #[source] nix::Error),
    #[cfg(unix)]
    #[error("Invalid chmod flag: {0}")]
    NixFlag(ModeBits),

    #[error(transparent)]
    DynError(#[from] Box<dyn 'static + std::error::Error + Sync + Send>),
}

#[derive(Debug, Error)]
pub enum ParamError {
    #[error("Parameter key/value was empty")]
    Empty,
    #[error("Invalid character found in key/value: {0}")]
    InvalidChar(char),
}
