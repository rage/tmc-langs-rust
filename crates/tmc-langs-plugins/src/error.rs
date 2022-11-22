//! Contains the PluginError type.

use std::path::PathBuf;
use tmc_langs_java::JavaError;

#[derive(thiserror::Error, Debug)]
pub enum PluginError {
    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[error("No matching plugin found in archive")]
    PluginNotFoundInArchive,
    #[error(transparent)]
    Tmc(#[from] tmc_langs_framework::TmcError),
    #[error(transparent)]
    Walkdir(#[from] walkdir::Error),
}

impl From<JavaError> for PluginError {
    fn from(e: JavaError) -> Self {
        Self::Tmc(e.into())
    }
}
