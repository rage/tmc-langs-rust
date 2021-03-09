//! Contains the crate error type

use std::path::PathBuf;
use thiserror::Error;

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
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    FileIo(#[from] FileIo),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Heim(#[from] heim::Error),

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

/// A wrapper for std::io::Error that provides more context for the failed operations.
#[derive(Error, Debug)]
pub enum FileIo {
    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read file at {0}")]
    FileRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file at {0}")]
    FileWrite(PathBuf, #[source] std::io::Error),
    #[error("Failed to create file at {0}")]
    FileCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove file at {0}")]
    FileRemove(PathBuf, #[source] std::io::Error),
    #[error("Failed to copy file from {from} to {to}")]
    FileCopy {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to move file from {from} to {to}")]
    FileMove {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to clone file handle")]
    FileHandleClone(#[source] std::io::Error),

    #[error("Failed to open directory at {0}")]
    DirOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read directory at {0}")]
    DirRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to create directory at {0}")]
    DirCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove directory at {0}")]
    DirRemove(PathBuf, #[source] std::io::Error),

    #[error("Failed to rename file {from} to {to}")]
    Rename {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to lock file at path {0}")]
    FdLock(PathBuf, #[source] std::io::Error),

    #[error("Path {0} has no file name")]
    NoFileName(PathBuf),
    #[error("Expected {0} to be a directory, but it was a file")]
    UnexpectedFile(PathBuf),
    #[error("Expected {0} to be a file")]
    UnexpectedNonFile(PathBuf),

    #[error("Directory walk error")]
    Walkdir(#[from] walkdir::Error),

    #[error("Failed to lock {0}: not a file or directory")]
    InvalidLockPath(PathBuf),

    // when there is no meaningful data that can be added to an error
    #[error("transparent")]
    Generic(#[from] std::io::Error),
}
