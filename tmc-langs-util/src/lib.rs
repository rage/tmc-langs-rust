//! Contains the task executor and a function for creating a tarball from a project.

pub mod tar;
pub mod task_executor;

pub use tmc_langs_abstraction::ValidationResult;
pub use tmc_langs_framework::{
    domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    Error, Result,
};

use tmc_langs_framework::plugin::LanguagePlugin;
use tmc_langs_python3::Python3Plugin;

const PLUGINS: [&dyn LanguagePlugin; 1] = [&Python3Plugin::new()];
