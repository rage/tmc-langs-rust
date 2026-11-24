#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Contains functionality for dealing with projects.

mod archive;
mod archive_safety;
mod command;
mod domain;
mod error;
mod meta_syntax;
mod plugin;
mod policy;
mod tmc_project_yml;

#[cfg(test)]
mod test_helpers;

pub use self::{
    archive::{Archive, ArchiveBuilder, Compression, Entry, entry_name},
    archive_safety::{EntryNameStyle, ExtractionBudget, ExtractionLimits, validate_entry_name},
    command::{ExitStatus, Output, TmcCommand},
    domain::{
        DEFAULT_TESTS_TIMEOUT, ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus,
        StyleValidationError, StyleValidationResult, StyleValidationStrategy, TIMEOUT_TEST_NAME,
        TestDesc, TestResult,
    },
    error::{CommandError, TmcError, describe_output},
    meta_syntax::{MetaString, MetaSyntaxParser},
    plugin::{Language, LanguagePlugin},
    policy::{EverythingIsStudentFilePolicy, NothingIsStudentFilePolicy, StudentFilePolicy},
    tmc_project_yml::{PythonVer, TmcProjectYml},
};
pub use nom;
pub use nom_language;
