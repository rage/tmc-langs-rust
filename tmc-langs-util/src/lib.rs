//! Contains the task executor

mod tar;
pub mod task_executor;

pub use tmc_langs_abstraction::ValidationResult;
pub use tmc_langs_framework::{
    domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus},
    plugin::Language,
    Error,
};
