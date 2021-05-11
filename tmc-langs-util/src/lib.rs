#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Contains various helpful utilities to be used throughout the tmc-langs project.

pub mod error;
pub mod file_util;
pub mod notification_reporter;
pub mod parse_util;
pub mod progress_reporter;

pub use error::FileError;
