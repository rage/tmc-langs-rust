//! Contains the task executor

pub mod error;
pub mod task_executor;

pub use error::UtilError;
pub use tmc_langs_framework::{
    domain::{
        ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, Strategy,
        ValidationResult,
    },
    plugin::Language,
};
