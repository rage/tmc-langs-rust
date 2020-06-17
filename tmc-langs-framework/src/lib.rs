//! Contains functionality for dealing with projects.

pub mod domain;
pub mod io;
pub mod plugin;
pub mod policy;

pub use plugin::LanguagePlugin;
pub use policy::StudentFilePolicy;

use domain::TmcProjectYml;
use io::zip;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // IO
    #[error("Failed to open file at {0}: {1}")]
    OpenFile(PathBuf, std::io::Error),
    #[error("Failed to create file at {0}: {1}")]
    CreateFile(PathBuf, std::io::Error),
    #[error("Failed to create dir at {0}: {1}")]
    CreateDir(PathBuf, std::io::Error),
    #[error("Failed to rename {0} to {1}: {2}")]
    Rename(PathBuf, PathBuf, std::io::Error),
    #[error("Failed to write to {0}: {1}")]
    Write(PathBuf, std::io::Error),

    #[error("Path {0} contained invalid UTF8")]
    UTF8(PathBuf),

    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip,
    #[error("Running command '{0}' failed")]
    CommandFailed(&'static str),

    #[error("Error in plugin: {0}")]
    Plugin(Box<dyn std::error::Error + 'static + Send + Sync>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    YamlDeserialization(#[from] serde_yaml::Error),
    #[error(transparent)]
    ZipError(#[from] zip::ZipError),
}

pub type Result<T> = std::result::Result<T, Error>;
