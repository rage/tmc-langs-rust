//! Contains functionality for dealing with projects.

pub mod domain;
pub mod io;
pub mod plugin;
pub mod policy;

use domain::TmcProjectYml;
use io::zip;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("No matching plugin found")]
    PluginNotFound,
    #[error("Error processing files")]
    FileProcessing(#[from] std::io::Error),
    #[error("Error deserializing YAML")]
    YamlDeserialization(#[from] serde_yaml::Error),
    #[error("Error reading or writing zip files")]
    ZipError(#[from] zip::ZipError),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip,
    #[error("Running command '{0}' failed")]
    CommandFailed(&'static str),
    #[error("Error in plugin: {0}")]
    Plugin(Box<dyn std::error::Error + 'static + Send + Sync>),
}

pub type Result<T> = std::result::Result<T, Error>;
