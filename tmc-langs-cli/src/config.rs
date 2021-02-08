//! Handles the CLI's configuration files and credentials.

mod credentials;
mod projects_config;
mod tmc_config;

pub use self::credentials::Credentials;
pub use self::projects_config::{CourseConfig, Exercise, ProjectsConfig};
pub use self::tmc_config::{ConfigValue, TmcConfig};
use crate::output::LocalExercise;

use anyhow::{Context, Error};
use std::env;
use std::path::PathBuf;

// base directory for a given plugin's settings files
fn get_tmc_dir(client_name: &str) -> Result<PathBuf, Error> {
    let config_dir = match env::var("TMC_LANGS_CONFIG_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::config_dir().context("Failed to find config directory")?,
    };
    Ok(config_dir.join(format!("tmc-{}", client_name)))
}

pub fn list_local_course_exercises(
    client_name: &str,
    course_slug: &str,
) -> Result<Vec<LocalExercise>, anyhow::Error> {
    let projects_dir = TmcConfig::load(client_name)?.projects_dir;
    let mut projects_config = ProjectsConfig::load(&projects_dir)?;

    let exercises = projects_config
        .courses
        .remove(course_slug)
        .map(|cc| cc.exercises)
        .unwrap_or_default();
    let mut local_exercises: Vec<LocalExercise> = vec![];
    for (exercise_slug, _) in exercises {
        local_exercises.push(LocalExercise {
            exercise_path: projects_dir.join(course_slug).join(&exercise_slug),
            exercise_slug,
        })
    }
    Ok(local_exercises)
}
