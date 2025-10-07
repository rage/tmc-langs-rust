#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Contains functionality for dealing with projects.

mod archive;
mod command;
mod domain;
mod error;
mod meta_syntax;
mod plugin;
mod policy;
mod tmc_project_yml;

pub use self::{
    archive::{Archive, ArchiveBuilder, Compression},
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
pub use nom_language;
