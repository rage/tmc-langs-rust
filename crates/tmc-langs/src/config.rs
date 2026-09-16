//! Handles the CLI's configuration files and credentials.

mod credentials;
mod mooc_credentials;
mod projects_config;
mod tmc_config;

pub(crate) use self::projects_config::simple_kebab_case;
pub use self::{
    credentials::Credentials,
    mooc_credentials::{MoocAuth, MoocAuthFailure, MoocCredentials},
    projects_config::{ProjectsConfig, ProjectsDirTmcExercise, TmcCourseConfig},
    tmc_config::TmcConfig,
};
use crate::{
    TMC_LANGS_CONFIG_DIR_VAR,
    data::{LocalExercise, LocalMoocExercise, LocalTmcExercise},
    error::LangsError,
};
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};
use tmc_langs_util::{
    FileError,
    file_util::{self, Lock, LockOptions},
};
use uuid::Uuid;

/// A process-wide lock for tests that set `TMC_LANGS_CONFIG_DIR` or the
/// bearer-token trust knob. Shared across the crate's test modules because the
/// env vars are process-wide: a per-module lock would not serialize them.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// base directory for a given plugin's settings files
pub(crate) fn get_tmc_dir(client_name: &str) -> Result<PathBuf, LangsError> {
    let config_dir = match env::var(TMC_LANGS_CONFIG_DIR_VAR) {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::config_dir().ok_or(LangsError::NoConfigDir)?,
    };
    Ok(config_dir.join(format!("tmc-{client_name}")))
}

/// Returns every exercise in the projects directory, from both backends, ordered
/// by course slug and then exercise slug within each backend.
///
/// This is the whole-projects-dir listing; [`list_local_tmc_course_exercises`]
/// and [`list_local_mooc_course_exercises`] are filtered views of it.
pub fn list_local_exercises(client_name: &str) -> Result<Vec<LocalExercise>, LangsError> {
    log::debug!("listing all local exercises for {client_name}");

    let projects_dir = TmcConfig::load(client_name)?.projects_dir;
    let projects_config = ProjectsConfig::load(&projects_dir)?;

    let mut local_exercises: Vec<LocalExercise> = vec![];

    let mut tmc_courses = projects_config.tmc_courses.iter().collect::<Vec<_>>();
    tmc_courses.sort_by_key(|(course_slug, _)| *course_slug);
    for (course_slug, course_config) in tmc_courses {
        for (exercise_slug, exercise) in &course_config.exercises {
            local_exercises.push(LocalExercise::Tmc(LocalTmcExercise {
                course_slug: course_slug.clone(),
                exercise_slug: exercise_slug.clone(),
                exercise_id: exercise.id,
                exercise_path: ProjectsConfig::get_tmc_exercise_download_target(
                    &projects_dir,
                    course_slug,
                    exercise_slug,
                ),
            }));
        }
    }

    let mut mooc_courses = projects_config.mooc_courses.values().collect::<Vec<_>>();
    mooc_courses.sort_by(|a, b| a.directory.cmp(&b.directory));
    for course_config in mooc_courses {
        for (exercise_id, exercise) in &course_config.exercises {
            local_exercises.push(LocalExercise::Mooc(LocalMoocExercise {
                course_slug: course_config.directory.clone(),
                course_id: course_config.course_id,
                exercise_slug: exercise.directory.clone(),
                exercise_id: *exercise_id,
                exercise_path: ProjectsConfig::get_mooc_exercise_download_target(
                    &projects_dir,
                    &course_config.directory,
                    &exercise.directory,
                ),
            }));
        }
    }

    Ok(local_exercises)
}

/// Returns all of the exercises for the given TMC course, identified by its
/// on-disk directory name.
pub fn list_local_tmc_course_exercises(
    client_name: &str,
    course_slug: &str,
) -> Result<Vec<LocalTmcExercise>, LangsError> {
    log::debug!("listing local course exercises of {course_slug} for {client_name}");

    Ok(list_local_exercises(client_name)?
        .into_iter()
        .filter_map(|exercise| match exercise {
            LocalExercise::Tmc(exercise) if exercise.course_slug == course_slug => Some(exercise),
            _ => None,
        })
        .collect())
}

/// Returns the local mooc exercises for the given course, looked up by course id
/// (mooc courses have no server-side slug, so the TMC slug-based lookup does not
/// apply).
pub fn list_local_mooc_course_exercises(
    client_name: &str,
    course_id: Uuid,
) -> Result<Vec<LocalMoocExercise>, LangsError> {
    log::debug!("listing local course exercises of {course_id} for {client_name}");

    Ok(list_local_exercises(client_name)?
        .into_iter()
        .filter_map(|exercise| match exercise {
            LocalExercise::Mooc(exercise) if exercise.course_id == course_id => Some(exercise),
            _ => None,
        })
        .collect())
}

/// Migrates an exercise from a location that's not managed by tmc-langs to the projects directory.
pub fn migrate_exercise(
    tmc_config: TmcConfig,
    course_slug: &str,
    exercise_slug: &str,
    exercise_id: u32,
    exercise_checksum: &str,
    exercise_path: &Path,
) -> Result<(), LangsError> {
    log::debug!(
        "migrating exercise {} from {}",
        exercise_id,
        exercise_path.display()
    );

    let mut lock = Lock::dir(exercise_path, LockOptions::Write)?;
    let _guard = lock.lock()?;
    let mut projects_config = ProjectsConfig::load(&tmc_config.projects_dir)?;
    let course_config = projects_config
        .tmc_courses
        .entry(course_slug.to_string())
        .or_insert(TmcCourseConfig {
            course: course_slug.to_string(),
            exercises: BTreeMap::new(),
        });

    let target_dir = ProjectsConfig::get_tmc_exercise_download_target(
        &tmc_config.projects_dir,
        course_slug,
        exercise_slug,
    );
    if target_dir.exists() {
        return Err(LangsError::DirectoryExists(target_dir));
    }

    course_config.exercises.insert(
        exercise_slug.to_string(),
        ProjectsDirTmcExercise {
            id: exercise_id,
            checksum: exercise_checksum.to_string(),
        },
    );

    super::move_dir(exercise_path, &target_dir)?;
    course_config.save_to_projects_dir(&tmc_config.projects_dir)?;
    Ok(())
}

/// Moves the projects directory from its current location to the target, taking all of the contained exercises with it.
pub fn move_projects_dir(mut tmc_config: TmcConfig, target: PathBuf) -> Result<(), LangsError> {
    log::debug!("moving projects dir to {}", target.display());

    if target.is_file() {
        return Err(FileError::UnexpectedFile(target).into());
    }
    if !target.exists() {
        file_util::create_dir_all(&target)?;
    }

    let target_canon = target
        .canonicalize()
        .map_err(|e| LangsError::Canonicalize(target.clone(), e))?;
    let prev_dir_canon = tmc_config
        .projects_dir
        .canonicalize()
        .map_err(|e| LangsError::Canonicalize(target.clone(), e))?;
    if target_canon == prev_dir_canon {
        return Err(LangsError::MovingProjectsDirToItself);
    }

    let old_projects_dir = tmc_config.set_projects_dir(target.clone())?;

    let mut lock = Lock::dir(old_projects_dir.clone(), LockOptions::Write)?;
    let _guard = lock.lock()?;
    super::move_dir(&old_projects_dir, &target)?;
    tmc_config.save()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use toml::value::Table;

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

    /// Points `TMC_LANGS_CONFIG_DIR` at `config_dir` and writes a config for
    /// client `test` whose projects dir is `projects_dir`. Callers must hold
    /// [`env_lock`].
    fn set_up_projects_dir(config_dir: &Path, projects_dir: &Path) {
        // SAFETY: all env access in these tests is serialized by `env_lock`.
        unsafe {
            env::set_var(TMC_LANGS_CONFIG_DIR_VAR, config_dir);
        }
        file_to(
            config_dir.join("tmc-test"),
            "config.toml",
            format!(
                "projects-dir = '{}'\n",
                projects_dir.display().to_string().replace('\\', "\\\\")
            ),
        );
    }

    /// Writes a TMC course with a single exercise, plus the exercise directory
    /// the config loader requires to consider it present.
    fn tmc_course_to(projects_dir: &Path, course_slug: &str, exercise_slug: &str, id: u32) {
        file_to(
            projects_dir.join("tmc").join(course_slug),
            "course_config.toml",
            format!(
                "course = '{course_slug}'\n\
                 [exercises.'{exercise_slug}']\n\
                 id = {id}\n\
                 checksum = 'abc'\n"
            ),
        );
        file_to(
            projects_dir
                .join("tmc")
                .join(course_slug)
                .join(exercise_slug),
            "src/main.py",
            "",
        );
    }

    /// Writes a mooc course with a single exercise. Mooc courses are keyed by
    /// instance id and live under `mooc/<directory>`.
    fn mooc_course_to(
        projects_dir: &Path,
        directory: &str,
        course_id: Uuid,
        exercise_slug: &str,
        exercise_id: Uuid,
    ) {
        file_to(
            projects_dir.join("mooc").join(directory),
            "course_config.toml",
            format!(
                "course_id = '{course_id}'\n\
                 instance_id = '{}'\n\
                 course = '{directory}'\n\
                 directory = '{directory}'\n\
                 [exercises.'{exercise_id}']\n\
                 name = '{exercise_slug}'\n\
                 task_id = '{}'\n\
                 checksum = 'abc'\n\
                 directory = '{exercise_slug}'\n",
                Uuid::nil(),
                Uuid::nil(),
            ),
        );
        file_to(
            projects_dir
                .join("mooc")
                .join(directory)
                .join(exercise_slug),
            "src/main.py",
            "",
        );
    }

    #[test]
    fn lists_local_tmc_course_exercises_with_ids() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let _guard = env_lock();

        let config_dir = tempfile::tempdir().unwrap();
        let projects_dir = tempfile::tempdir().unwrap();
        set_up_projects_dir(config_dir.path(), projects_dir.path());
        tmc_course_to(projects_dir.path(), "some-course", "some-exercise", 1234);

        let exercises = list_local_tmc_course_exercises("test", "some-course").unwrap();

        assert_eq!(exercises.len(), 1);
        assert_eq!(exercises[0].course_slug, "some-course");
        assert_eq!(exercises[0].exercise_slug, "some-exercise");
        assert_eq!(exercises[0].exercise_id, 1234);
        assert_eq!(
            exercises[0].exercise_path,
            projects_dir.path().join("tmc/some-course/some-exercise")
        );
    }

    #[test]
    fn lists_local_exercises_from_both_backends() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let _guard = env_lock();

        let config_dir = tempfile::tempdir().unwrap();
        let projects_dir = tempfile::tempdir().unwrap();
        set_up_projects_dir(config_dir.path(), projects_dir.path());
        tmc_course_to(projects_dir.path(), "b-course", "tmc-exercise", 7);
        tmc_course_to(projects_dir.path(), "a-course", "other-exercise", 8);
        let course_id = Uuid::from_u128(1);
        let exercise_id = Uuid::from_u128(2);
        mooc_course_to(
            projects_dir.path(),
            "mooc-course",
            course_id,
            "mooc-exercise",
            exercise_id,
        );

        let exercises = list_local_exercises("test").unwrap();

        assert_eq!(exercises.len(), 3);
        let LocalExercise::Tmc(first) = &exercises[0] else {
            panic!("expected the tmc courses first, in slug order: {exercises:?}");
        };
        assert_eq!(first.course_slug, "a-course");
        let LocalExercise::Mooc(mooc) = &exercises[2] else {
            panic!("expected the mooc course last: {exercises:?}");
        };
        assert_eq!(mooc.course_slug, "mooc-course");
        assert_eq!(mooc.course_id, course_id);
        assert_eq!(mooc.exercise_id, exercise_id);
        assert_eq!(mooc.exercise_slug, "mooc-exercise");
        assert_eq!(
            mooc.exercise_path,
            projects_dir.path().join("mooc/mooc-course/mooc-exercise")
        );
    }

    /// The per-course commands are filtered views of the whole-dir listing, so
    /// they must not leak the other backend's or another course's exercises.
    #[test]
    fn per_course_listings_filter_the_full_listing() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let _guard = env_lock();

        let config_dir = tempfile::tempdir().unwrap();
        let projects_dir = tempfile::tempdir().unwrap();
        set_up_projects_dir(config_dir.path(), projects_dir.path());
        tmc_course_to(projects_dir.path(), "wanted", "tmc-exercise", 7);
        tmc_course_to(projects_dir.path(), "unwanted", "other-exercise", 8);
        let course_id = Uuid::from_u128(1);
        mooc_course_to(
            projects_dir.path(),
            "wanted-mooc",
            course_id,
            "mooc-exercise",
            Uuid::from_u128(2),
        );
        mooc_course_to(
            projects_dir.path(),
            "unwanted-mooc",
            Uuid::from_u128(3),
            "other-mooc-exercise",
            Uuid::from_u128(4),
        );

        let tmc = list_local_tmc_course_exercises("test", "wanted").unwrap();
        assert_eq!(tmc.len(), 1);
        assert_eq!(tmc[0].exercise_slug, "tmc-exercise");

        let mooc = list_local_mooc_course_exercises("test", course_id).unwrap();
        assert_eq!(mooc.len(), 1);
        assert_eq!(mooc[0].exercise_slug, "mooc-exercise");
    }

    #[test]
    fn migrates() {
        init();
        crate::test_util::ensure_isolated_locks_dir();

        let projects_dir = tempfile::tempdir().unwrap();
        let exercise_path = tempfile::tempdir().unwrap();

        let tmc_config = TmcConfig {
            location: PathBuf::new(),
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };

        file_to(&exercise_path, "some_file", "");

        assert!(
            !projects_dir
                .path()
                .join("tmc/course/exercise/some_file")
                .exists()
        );

        migrate_exercise(
            tmc_config,
            "course",
            "exercise",
            0,
            "checksum",
            exercise_path.path(),
        )
        .unwrap();

        assert!(
            projects_dir
                .path()
                .join("tmc/course/exercise/some_file")
                .exists()
        );

        assert!(!exercise_path.path().exists());
    }

    #[test]
    fn moves_projects_dir() {
        init();
        crate::test_util::ensure_isolated_locks_dir();

        // can't use a tempfile for the config location directly
        // because windows won't let us replace a tempfile while it's "open"
        let config_dir = tempfile::tempdir().unwrap();
        let config_location = config_dir.path().join("tmc_config.temp");
        let projects_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        let tmc_config = TmcConfig {
            location: config_location,
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };

        file_to(
            projects_dir.path(),
            "some course/some exercise/some file",
            "",
        );

        assert!(
            !target_dir
                .path()
                .join("some course/some exercise/some file")
                .exists()
        );

        move_projects_dir(tmc_config, target_dir.path().to_path_buf()).unwrap();

        assert!(
            target_dir
                .path()
                .join("some course/some exercise/some file")
                .exists()
        );
        assert!(!projects_dir.path().exists());
    }
}
