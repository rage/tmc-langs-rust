//! Contains the task executor

pub mod task_executor;

pub use tmc_langs_framework::{
    domain::{
        ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, Strategy,
        ValidationResult,
    },
    plugin::Language,
    TmcError,
};
