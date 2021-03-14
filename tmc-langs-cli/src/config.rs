//! Handles the CLI's configuration files and credentials.

mod credentials;
mod projects_config;
mod tmc_config;

pub use self::credentials::Credentials;
pub use self::projects_config::{CourseConfig, Exercise, ProjectsConfig};
pub use self::tmc_config::{ConfigValue, TmcConfig};
use crate::output::LocalExercise;

use anyhow::{Context, Error};
use std::path::{Path, PathBuf};
use std::{collections::BTreeMap, env, fs};
use tmc_langs_util::file_util;

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
    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
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

pub fn migrate(
    tmc_config: &TmcConfig,
    course_slug: &str,
    exercise_slug: &str,
    exercise_id: usize,
    exercise_checksum: &str,
    exercise_path: &Path,
) -> anyhow::Result<()> {
    let mut lock = file_util::FileLock::new(exercise_path.to_path_buf())?;
    let guard = lock.lock()?;

    let mut projects_config = ProjectsConfig::load(&tmc_config.projects_dir)?;
    let course_config = projects_config
        .courses
        .entry(course_slug.to_string())
        .or_insert(CourseConfig {
            course: course_slug.to_string(),
            exercises: BTreeMap::new(),
        });

    let target_dir = ProjectsConfig::get_exercise_download_target(
        &tmc_config.projects_dir,
        course_slug,
        exercise_slug,
    );
    if target_dir.exists() {
        anyhow::bail!(
            "Tried to migrate exercise to {}; however, something already exists at that path.",
            target_dir.display()
        );
    }

    course_config.exercises.insert(
        exercise_slug.to_string(),
        Exercise {
            id: exercise_id,
            checksum: exercise_checksum.to_string(),
        },
    );

    super::move_dir(exercise_path, guard, &target_dir)?;
    course_config.save_to_projects_dir(&tmc_config.projects_dir)?;
    Ok(())
}

pub fn move_projects_dir(
    mut tmc_config: TmcConfig,
    config_path: &Path,
    target: PathBuf,
) -> anyhow::Result<()> {
    if target.is_file() {
        anyhow::bail!("The target path points to a file.")
    }
    if !target.exists() {
        fs::create_dir_all(&target)
            .with_context(|| format!("Failed to create directory at {}", target.display()))?;
    }

    let target_canon = target
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize {}", target.display()))?;
    let prev_dir_canon = tmc_config.projects_dir.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize {}",
            tmc_config.projects_dir.display()
        )
    })?;
    if target_canon == prev_dir_canon {
        anyhow::bail!("Attempted to move the projects-dir to the directory it's already in.")
    }

    let old_projects_dir = tmc_config.set_projects_dir(target.clone())?;

    let mut lock = file_util::FileLock::new(old_projects_dir.clone())?;
    let guard = lock.lock()?;

    super::move_dir(&old_projects_dir, guard, &target)?;
    tmc_config.save(config_path)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use toml::value::Table;

    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    #[test]
    fn migrates() {
        init();

        let projects_dir = tempfile::tempdir().unwrap();
        let exercise_path = tempfile::tempdir().unwrap();

        let tmc_config = TmcConfig {
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };

        file_to(&exercise_path, "some_file", "");

        assert!(!projects_dir
            .path()
            .join("course/exercise/some_file")
            .exists());

        migrate(
            &tmc_config,
            "course",
            "exercise",
            0,
            "checksum",
            exercise_path.path(),
        )
        .unwrap();

        assert!(projects_dir
            .path()
            .join("course/exercise/some_file")
            .exists());

        assert!(!exercise_path.path().exists());
    }

    #[test]
    fn moves_projects_dir() {
        init();

        let projects_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        let config_path = tempfile::NamedTempFile::new().unwrap();
        let tmc_config = TmcConfig {
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };

        file_to(
            projects_dir.path(),
            "some course/some exercise/some file",
            "",
        );

        assert!(!target_dir
            .path()
            .join("some course/some exercise/some file")
            .exists());

        move_projects_dir(
            tmc_config,
            config_path.path(),
            target_dir.path().to_path_buf(),
        )
        .unwrap();

        assert!(target_dir
            .path()
            .join("some course/some exercise/some file")
            .exists());
        assert!(!projects_dir.path().exists());
    }
}
