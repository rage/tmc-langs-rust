//! Contains functionality for dealing with projects.

mod command;
mod domain;
mod error;
mod meta_syntax;
mod plugin;
mod policy;
mod tmc_project_yml;

pub use self::command::{ExitStatus, Output, TmcCommand};
pub use self::domain::{
    ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, StyleValidationError,
    StyleValidationResult, StyleValidationStrategy, TestDesc, TestResult,
};
pub use self::error::{CommandError, PopenError, TmcError};
pub use self::meta_syntax::{MetaString, MetaSyntaxParser};
pub use self::plugin::{Language, LanguagePlugin};
pub use self::policy::{
    EverythingIsStudentFilePolicy, NothingIsStudentFilePolicy, StudentFilePolicy,
};
pub use self::tmc_project_yml::{PythonVer, TmcProjectYml};
pub use nom;
