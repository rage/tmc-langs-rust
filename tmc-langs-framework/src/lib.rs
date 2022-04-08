#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Contains functionality for dealing with projects.

mod command;
mod domain;
mod error;
mod meta_syntax;
mod plugin;
mod policy;
mod tmc_project_yml;

pub use self::{
    command::{ExitStatus, Output, TmcCommand},
    domain::{
        ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, StyleValidationError,
        StyleValidationResult, StyleValidationStrategy, TestDesc, TestResult,
    },
    error::{CommandError, PopenError, TmcError},
    meta_syntax::{MetaString, MetaSyntaxParser},
    plugin::{Language, LanguagePlugin},
    policy::{EverythingIsStudentFilePolicy, NothingIsStudentFilePolicy, StudentFilePolicy},
    tmc_project_yml::{PythonVer, TmcProjectYml},
};
pub use nom;
use serde::Deserialize;
use std::{fmt::Display, str::FromStr};
use ts_rs::TS;

/// Supported compression methods.
#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub enum Compression {
    /// .tar
    Tar,
    /// .zip
    Zip,
    /// .tar.ztd
    TarZstd,
}

impl Display for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tar => write!(f, "tar"),
            Self::Zip => write!(f, "zip"),
            Self::TarZstd => write!(f, "zstd"),
        }
    }
}

impl FromStr for Compression {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let format = match s {
            "tar" => Compression::Tar,
            "zip" => Compression::Zip,
            "zstd" => Compression::TarZstd,
            _ => return Err("invalid format"),
        };
        Ok(format)
    }
}
