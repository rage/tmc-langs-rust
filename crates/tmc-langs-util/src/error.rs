//! Contains the FileError error type for file operations.

use std::path::PathBuf;
use thiserror::Error;

/// A wrapper for std::io::Error that provides more context for the failed operations.
#[derive(Error, Debug)]
pub enum FileError {
    // file_util errors
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
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
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
    #[error("Path {0} has no file name")]
    NoFileName(PathBuf),
    #[error("Expected {0} to be a directory, but it was a file")]
    UnexpectedFile(PathBuf),
    #[error("Failed to write data")]
    WriteError(#[source] std::io::Error),
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),

    // lock errors
    #[error("Failed to lock file at path {0}")]
    FdLock(PathBuf, #[source] std::io::Error),
    #[error("Failed to lock {0}: not a file or directory")]
    InvalidLockPath(PathBuf),

    #[error("Directory walk error")]
    Walkdir(#[from] walkdir::Error),
}
