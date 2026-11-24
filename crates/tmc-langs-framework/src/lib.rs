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
mod process_tree;
mod tmc_project_yml;
mod tool_lookup;

#[cfg(test)]
mod test_helpers;

pub use self::{
    archive::{Archive, ArchiveBuilder, Compression, Entry, entry_name},
    archive_safety::{EntryNameStyle, ExtractionBudget, ExtractionLimits, validate_entry_name},
    command::{
        BUILD_TIMEOUT, ExitStatus, Output, TmcCommand, batch_command_line,
        output_shows_blocked_executable,
    },
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
    tool_lookup::{ResolvedTool, absolute_tool_path, missing_tool_hint, resolve_tool},
};
pub use nom;
pub use nom_language;
