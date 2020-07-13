use crate::io::zip;

use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // IO
    #[error("Failed to open file at {0}")]
    OpenFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to create file at {0}")]
    CreateFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to create dir(s) at {0}")]
    CreateDir(PathBuf, #[source] std::io::Error),
    #[error("Failed to rename {0} to {1}")]
    Rename(PathBuf, PathBuf, #[source] std::io::Error),
    #[error("Failed to write to {0}")]
    Write(PathBuf, #[source] std::io::Error),

    #[error("Path {0} contained invalid UTF8")]
    UTF8(PathBuf),

    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip,
    #[error("Running command '{0}' failed")]
    CommandFailed(&'static str, #[source] std::io::Error),

    #[error("Failed to spawn command: {0}")]
    CommandSpawn(&'static str, #[source] std::io::Error),
    #[error("Test timed out")]
    TestTimeout,

    #[error("Error in plugin: {0}")]
    Plugin(#[source] Box<dyn std::error::Error + 'static + Send + Sync>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    YamlDeserialization(#[from] serde_yaml::Error),
    #[error(transparent)]
    ZipError(#[from] zip::ZipError),
}
