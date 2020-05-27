//! Contains the task executor and a function for creating a tarball from a project.

pub mod tar;
pub mod task_executor;

pub use tmc_langs_abstraction::ValidationResult;
pub use tmc_langs_framework::{
    domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    Error,
};

use tmc_langs_framework::plugin::LanguagePlugin;
use tmc_langs_java::ant::AntPlugin;
use tmc_langs_java::maven::MavenPlugin;
use tmc_langs_python3::Python3Plugin;

fn get_plugins() -> Result<Vec<Box<dyn LanguagePlugin>>, Error> {
    Ok(vec![
        Box::new(Python3Plugin::new()),
        Box::new(MavenPlugin::new()?),
        Box::new(AntPlugin::new()?),
    ])
}
