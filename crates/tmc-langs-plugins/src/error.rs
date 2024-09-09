//! Contains the PluginError type.

use std::path::PathBuf;
// the Java plugin is disabled on musl
#[cfg(not(target_env = "musl"))]
use tmc_langs_java::JavaError;

#[derive(thiserror::Error, Debug)]
pub enum PluginError {
    // on musl, warn the user about the Java plugin being nonfunctional
    #[cfg(not(target_env = "musl"))]
    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[cfg(target_env = "musl")]
    #[error("No matching plugin found for {0}. Note that Java support is disabled on musl.")]
    PluginNotFound(PathBuf),
    #[error("No matching plugin found in archive.")]
    PluginNotFoundInArchive,
    #[error(transparent)]
    Tmc(#[from] tmc_langs_framework::TmcError),
    #[error(transparent)]
    Walkdir(#[from] walkdir::Error),
}

// the Java plugin is disabled on musl
#[cfg(not(target_env = "musl"))]
impl From<JavaError> for PluginError {
    fn from(e: JavaError) -> Self {
        Self::Tmc(e.into())
    }
}
