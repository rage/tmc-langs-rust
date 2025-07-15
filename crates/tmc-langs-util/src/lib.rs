#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Contains various helpful utilities to be used throughout the tmc-langs project.

pub mod deserialize;
pub mod error;
pub mod file_util;
pub mod notification_reporter;
pub mod parse_util;
pub mod path_util;
pub mod progress_reporter;
pub mod serialize;

pub use error::FileError;
use thiserror::Error;

pub type JsonError = serde_path_to_error::Error<serde_json::Error>;
#[derive(Debug, Error)]
#[error(transparent)]
pub enum TomlError {
    Path(#[from] serde_path_to_error::Error<toml::de::Error>),
    Toml(#[from] toml::de::Error),
}
pub type YamlError = serde_path_to_error::Error<serde_yaml::Error>;
