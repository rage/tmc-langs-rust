//! Handles the CLI's configuration files and credentials.

mod credentials;
mod mooc_credentials;
mod projects_config;
mod tmc_config;

pub(crate) use self::projects_config::MoocCourseConfig;
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
    ffi::OsStr,
    path::{Path, PathBuf},
};
use tmc_langs_util::{
    FileError,
    file_util::{self, Lock, LockOptions},
};
use uuid::Uuid;

/// Serializes tests that set `TMC_LANGS_CONFIG_DIR` or the bearer-token trust knob.
/// Crate-wide because a per-module lock would not serialize process-wide env vars.
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
/// by course slug and then exercise slug within each backend. No two have the same path: of
/// mooc exercises recorded in one directory, only the first by id is kept.
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
                course_id: course_config.course_id,
                exercise_slug: exercise_slug.clone(),
                exercise_id: exercise.id,
                exercise_path: ProjectsConfig::resolve_tmc_exercise_path(
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
/// (mooc courses have no slug, unlike [`list_local_tmc_course_exercises`]).
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
            course_id: None,
            exercises: BTreeMap::new(),
        });

    let target_dir = ProjectsConfig::get_tmc_exercise_download_target(
        &tmc_config.projects_dir,
        course_slug,
        exercise_slug,
    );
    let existing_dir = ProjectsConfig::resolve_tmc_exercise_path(
        &tmc_config.projects_dir,
        course_slug,
        exercise_slug,
    );
    if existing_dir.exists() {
        return Err(LangsError::DirectoryExists(existing_dir));
    }

    course_config.exercises.insert(
        exercise_slug.to_string(),
        ProjectsDirTmcExercise {
            id: exercise_id,
            checksum: exercise_checksum.to_string(),
        },
    );

    super::move_dir(exercise_path, &target_dir, || {
        course_config.save_to_projects_dir(&tmc_config.projects_dir)
    })
}

/// Fails with the error [`move_projects_dir`] would, without moving anything, so a caller can
/// refuse before it closes editors.
pub fn check_move_projects_dir(tmc_config: &TmcConfig, target: &Path) -> Result<(), LangsError> {
    if target.is_file() {
        return Err(FileError::UnexpectedFile(target.to_path_buf()).into());
    }
    if tmc_config.projects_dir.is_dir() {
        ensure_outside_projects_dir(&tmc_config.projects_dir, target)?;
    }
    Ok(())
}

/// Moves the projects directory from its current location to the target, taking all of the contained exercises with it.
pub fn move_projects_dir(mut tmc_config: TmcConfig, target: PathBuf) -> Result<(), LangsError> {
    log::debug!("moving projects dir to {}", target.display());

    check_move_projects_dir(&tmc_config, &target)?;

    if !tmc_config.projects_dir.is_dir() {
        // The only way out of `ProjectsDirUnreachable`: there is nothing left to move.
        file_util::create_dir_all(&target)?;
        tmc_config.set_projects_dir(target)?;
        return tmc_config.save();
    }

    file_util::create_dir_all(&target)?;

    let old_projects_dir = tmc_config.set_projects_dir(target.clone())?;

    let mut lock = Lock::dir(old_projects_dir.clone(), LockOptions::Write)?;
    let _guard = lock.lock()?;
    super::move_dir(&old_projects_dir, &target, || tmc_config.save())
}

/// Files operating systems put in any folder, which do not stop a restore.
const OS_METADATA_FILE_NAMES: &[&str] = &[".ds_store", "thumbs.db", "desktop.ini"];

/// Moves every entry of the projects directory into `target`, leaving the projects directory
/// empty; [`restore_projects`] moves them back. `target` must be absent or empty, outside the
/// projects directory and on the same file system. Moves nothing if any entry cannot be moved.
pub fn set_projects_aside(tmc_config: &TmcConfig, target: &Path) -> Result<(), LangsError> {
    log::debug!("setting the projects aside into {}", target.display());

    let projects_dir = reachable_projects_dir(tmc_config)?;
    ensure_outside_projects_dir(projects_dir, target)?;
    if !target.exists() {
        file_util::create_dir_all(target)?;
    } else if !target.is_dir() {
        return Err(FileError::UnexpectedFile(target.to_path_buf()).into());
    } else if file_util::read_dir(target)?.next().is_some() {
        return Err(LangsError::NonEmptyDir(target.to_path_buf()));
    }

    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
    let mut lock = Lock::dir(projects_dir, LockOptions::Write)?;
    let _guard = lock.lock()?;
    super::move_all_dir_entries(projects_dir, target)
}

/// Moves every entry of `source`, a folder [`set_projects_aside`] filled, back into the projects
/// directory and deletes `source`. Fails with [`LangsError::RestoreConflict`], moving nothing,
/// unless the projects directory holds nothing but OS metadata files such as `.DS_Store`, since
/// courses merged with another account's would be submitted as that account's. A metadata file
/// in both is kept from the projects directory. A missing projects directory whose parent exists
/// is created, since it held nothing to conflict with.
pub fn restore_projects(tmc_config: &TmcConfig, source: &Path) -> Result<(), LangsError> {
    log::debug!("restoring the projects set aside in {}", source.display());

    let projects_dir = tmc_config.get_projects_dir();
    if !projects_dir.exists() && projects_dir.parent().is_some_and(Path::is_dir) {
        file_util::create_dir_all(projects_dir)?;
    }
    let projects_dir = reachable_projects_dir(tmc_config)?;

    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
    let mut lock = Lock::dir(projects_dir, LockOptions::Write)?;
    let _guard = lock.lock()?;
    ensure_outside_projects_dir(projects_dir, source)?;
    let mut present = super::dir_entry_names(projects_dir)?;
    present.sort();
    if let Some(entry) = present.iter().find(|name| !is_os_metadata_file(name)) {
        return Err(LangsError::RestoreConflict(projects_dir.join(entry)));
    }
    present.retain(|name| source.join(name).symlink_metadata().is_ok());
    for name in present {
        file_util::remove_file(source.join(name))?;
    }
    super::move_all_dir_entries(source, projects_dir)?;
    if let Err(err) = file_util::remove_dir_empty(source) {
        log::warn!(
            "restored the projects but could not delete {}: {err}",
            source.display()
        );
    }
    Ok(())
}

fn is_os_metadata_file(name: &OsStr) -> bool {
    OS_METADATA_FILE_NAMES.contains(&name.to_string_lossy().to_lowercase().as_str())
}

fn reachable_projects_dir(tmc_config: &TmcConfig) -> Result<&Path, LangsError> {
    let projects_dir = tmc_config.get_projects_dir();
    if !projects_dir.is_dir() {
        return Err(LangsError::ProjectsDirUnreachable(
            projects_dir.to_path_buf(),
        ));
    }
    Ok(projects_dir)
}

/// Fails unless `dir`, which need not exist yet, neither is, is inside nor contains
/// `projects_dir`.
fn ensure_outside_projects_dir(projects_dir: &Path, dir: &Path) -> Result<(), LangsError> {
    let dir_canon = canonicalize_maybe_missing(dir)?;
    let projects_dir_canon = file_util::canonicalize(projects_dir)?;
    if dir_canon == projects_dir_canon {
        return Err(LangsError::MovingProjectsDirToItself);
    }
    if dir_canon.starts_with(&projects_dir_canon) || projects_dir_canon.starts_with(&dir_canon) {
        return Err(LangsError::MovingProjectsDirIntoItself);
    }
    Ok(())
}

/// `path` canonicalized through its nearest existing ancestor, with the missing rest appended.
fn canonicalize_maybe_missing(path: &Path) -> Result<PathBuf, LangsError> {
    let absolute =
        std::path::absolute(path).map_err(|e| LangsError::Canonicalize(path.to_path_buf(), e))?;
    let existing = absolute
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .unwrap_or(&absolute);
    let canon = file_util::canonicalize(existing)?;
    match absolute.strip_prefix(existing) {
        Ok(missing) if !missing.as_os_str().is_empty() => Ok(canon.join(missing)),
        _ => Ok(canon),
    }
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

    /// tmc-vscode reads these locations without starting the CLI, in `src/init/cliConfigFolder.ts`.
    #[test]
    fn config_files_stay_where_tmc_vscode_looks_for_them() {
        let _env = env_lock();
        let previous = env::var_os(TMC_LANGS_CONFIG_DIR_VAR);
        // SAFETY: all env access in these tests is serialized by `env_lock`.
        unsafe {
            env::remove_var(TMC_LANGS_CONFIG_DIR_VAR);
        }
        let default_dir = get_tmc_dir("client");
        let config_dir = tempfile::tempdir().unwrap();
        // SAFETY: as above.
        unsafe {
            env::set_var(TMC_LANGS_CONFIG_DIR_VAR, config_dir.path());
        }
        let configured_dir = get_tmc_dir("client").unwrap();
        let config_file = TmcConfig::get_location("client").unwrap();
        let credentials_file = Credentials::get_credentials_path("client").unwrap();
        // SAFETY: as above.
        unsafe {
            match previous {
                Some(previous) => env::set_var(TMC_LANGS_CONFIG_DIR_VAR, previous),
                None => env::remove_var(TMC_LANGS_CONFIG_DIR_VAR),
            }
        }

        assert_eq!(
            default_dir.unwrap(),
            dirs::config_dir().unwrap().join("tmc-client")
        );
        assert_eq!(configured_dir, config_dir.path().join("tmc-client"));
        assert_eq!(config_file, configured_dir.join("config.toml"));
        assert_eq!(credentials_file, configured_dir.join("credentials.json"));
        let mooc_credentials_file = MoocCredentials::credentials_file_name();
        assert!(
            mooc_credentials_file == "credentials_mooc.json"
                || (mooc_credentials_file.starts_with("credentials_mooc_")
                    && mooc_credentials_file.ends_with(".json")),
            "{mooc_credentials_file}"
        );
    }

    /// Writes a TMC course with a single exercise, plus the exercise directory
    /// the config loader requires to consider it present.
    fn tmc_course_to(projects_dir: &Path, course_slug: &str, exercise_slug: &str, id: u32) {
        file_to(
            projects_dir.join(course_slug),
            "course_config.toml",
            format!(
                "course = '{course_slug}'\n\
                 [exercises.'{exercise_slug}']\n\
                 id = {id}\n\
                 checksum = 'abc'\n"
            ),
        );
        file_to(
            projects_dir.join(course_slug).join(exercise_slug),
            "src/main.py",
            "",
        );
    }

    /// Writes a mooc course with a single exercise under `mooc/<directory>`.
    /// `course_id` doubles as the instance id, which `ProjectsConfig` keys
    /// courses by, so courses sharing one would overwrite each other.
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
                 instance_id = '{course_id}'\n\
                 course = '{directory}'\n\
                 directory = '{directory}'\n\
                 [exercises.'{exercise_id}']\n\
                 name = '{exercise_slug}'\n\
                 task_id = '{}'\n\
                 checksum = 'abc'\n\
                 directory = '{exercise_slug}'\n",
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
            projects_dir.path().join("some-course/some-exercise")
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
    fn lists_tmc_exercises_from_the_legacy_and_split_layouts() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let _guard = env_lock();

        let config_dir = tempfile::tempdir().unwrap();
        let projects_dir = tempfile::tempdir().unwrap();
        set_up_projects_dir(config_dir.path(), projects_dir.path());
        file_to(
            projects_dir.path(),
            "course/course_config.toml",
            "course = 'course'\n\
             [exercises.legacy]\nid = 1\nchecksum = 'a'\n\
             [exercises.split]\nid = 2\nchecksum = 'b'\n",
        );
        file_to(projects_dir.path(), "course/legacy/src/main.py", "");
        file_to(projects_dir.path(), "tmc/course/split/src/main.py", "");

        let exercises = list_local_tmc_course_exercises("test", "course").unwrap();

        let paths = exercises
            .iter()
            .map(|e| (e.exercise_slug.as_str(), e.exercise_path.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                ("legacy", projects_dir.path().join("course/legacy")),
                ("split", projects_dir.path().join("tmc/course/split")),
            ]
        );
    }

    #[test]
    fn migrate_refuses_an_exercise_already_in_the_split_layout() {
        init();
        crate::test_util::ensure_isolated_locks_dir();

        let projects_dir = tempfile::tempdir().unwrap();
        let exercise_path = tempfile::tempdir().unwrap();
        file_to(&exercise_path, "some_file", "");
        file_to(projects_dir.path(), "tmc/course/exercise/existing", "");
        let tmc_config = TmcConfig {
            location: PathBuf::new(),
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };

        let err = migrate_exercise(
            tmc_config,
            "course",
            "exercise",
            0,
            "checksum",
            exercise_path.path(),
        )
        .unwrap_err();

        assert!(matches!(err, LangsError::DirectoryExists(_)), "{err:?}");
        assert!(exercise_path.path().join("some_file").exists());
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
                .join("course/exercise/some_file")
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
                .join("course/exercise/some_file")
                .exists()
        );

        assert!(!exercise_path.path().exists());
    }

    fn config_with_projects_dir(projects_dir: &Path) -> TmcConfig {
        TmcConfig {
            location: projects_dir.join("unused-config.toml"),
            projects_dir: projects_dir.to_path_buf(),
            table: Table::new(),
        }
    }

    fn entry_names(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn projects_set_aside_come_back_on_restore() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let projects_dir = tempfile::tempdir().unwrap();
        let config = config_with_projects_dir(projects_dir.path());
        file_to(projects_dir.path(), "course/exercise/file", "contents");
        file_to(projects_dir.path(), "mooc/course/course_config.toml", "");
        file_to(projects_dir.path(), ".DS_Store", "");
        let aside = tempfile::tempdir().unwrap();
        let target = aside.path().join("set-aside");

        set_projects_aside(&config, &target).unwrap();
        assert!(entry_names(projects_dir.path()).is_empty());
        assert_eq!(entry_names(&target), [".DS_Store", "course", "mooc"]);

        file_to(projects_dir.path(), ".DS_Store", "");
        restore_projects(&config, &target).unwrap();
        assert_eq!(
            entry_names(projects_dir.path()),
            [".DS_Store", "course", "mooc"]
        );
        let file = projects_dir.path().join("course/exercise/file");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "contents");
        assert!(!target.exists());
    }

    /// Tells the child spawned by [`setting_projects_aside_takes_locks_in_the_download_order`]
    /// which projects directory to lock and where to signal.
    const HOLD_CONFIG_UPDATE_LOCK_VAR: &str = "TMC_TEST_HOLD_CONFIG_UPDATE_LOCK";
    const SIGNAL_DIR_VAR: &str = "TMC_TEST_SIGNAL_DIR";

    fn wait_for(path: &Path) {
        let waiting_since = std::time::Instant::now();
        while !path.exists() {
            assert!(
                waiting_since.elapsed() < std::time::Duration::from_secs(60),
                "{} never appeared",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Plays a download in a separate process, since `fcntl` locks never contend within one:
    /// holds the course config update lock, then reads the projects directory.
    #[test]
    fn child_process_holds_the_course_config_update_lock() {
        let (Ok(projects_dir), Ok(signal_dir)) = (
            env::var(HOLD_CONFIG_UPDATE_LOCK_VAR),
            env::var(SIGNAL_DIR_VAR),
        ) else {
            return;
        };
        let (projects_dir, signal_dir) = (PathBuf::from(projects_dir), PathBuf::from(signal_dir));
        let mut config_update_lock =
            ProjectsConfig::lock_course_config_updates(&projects_dir).unwrap();
        let _config_update_guard = config_update_lock.lock().unwrap();
        std::fs::write(signal_dir.join("held"), "").unwrap();
        wait_for(&signal_dir.join("go"));

        let mut projects_lock = Lock::dir(&projects_dir, LockOptions::Read).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let outcome = loop {
            if projects_lock.try_lock().unwrap().is_some() {
                break "free";
            }
            if std::time::Instant::now() > deadline {
                break "blocked";
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        std::fs::write(signal_dir.join("outcome"), outcome).unwrap();
    }

    #[test]
    fn setting_projects_aside_takes_locks_in_the_download_order() {
        init();
        let locks_dir = crate::test_util::ensure_isolated_locks_dir();
        let projects_dir = tempfile::tempdir().unwrap();
        let config = config_with_projects_dir(projects_dir.path());
        file_to(projects_dir.path(), "course/exercise/file", "contents");
        let aside = tempfile::tempdir().unwrap();
        let target = aside.path().join("set-aside");
        let signal_dir = tempfile::tempdir().unwrap();

        let mut child = std::process::Command::new(env::current_exe().unwrap())
            .args([
                "config::test::child_process_holds_the_course_config_update_lock",
                "--exact",
            ])
            .env(tmc_langs_util::file_util::LOCKS_DIR_ENV, locks_dir)
            .env(HOLD_CONFIG_UPDATE_LOCK_VAR, projects_dir.path())
            .env(SIGNAL_DIR_VAR, signal_dir.path())
            .spawn()
            .unwrap();
        wait_for(&signal_dir.path().join("held"));

        std::thread::scope(|scope| {
            let set_aside = scope.spawn(|| set_projects_aside(&config, &target));
            // lets the set-aside reach its locks before the child asks for the projects dir
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::fs::write(signal_dir.path().join("go"), "").unwrap();
            wait_for(&signal_dir.path().join("outcome"));
            assert!(child.wait().unwrap().success());
            set_aside.join().unwrap().unwrap();
        });

        let outcome = std::fs::read_to_string(signal_dir.path().join("outcome")).unwrap();
        assert_eq!(outcome, "free");
        assert_eq!(entry_names(&target), ["course"]);
    }

    #[test]
    fn a_restore_refuses_a_projects_dir_that_holds_exercises() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let projects_dir = tempfile::tempdir().unwrap();
        let config = config_with_projects_dir(projects_dir.path());
        let aside = tempfile::tempdir().unwrap();
        file_to(aside.path(), "course/old", "");
        file_to(aside.path(), "other/old", "");

        for existing in ["course/new", "another course/new"] {
            file_to(projects_dir.path(), existing, "");
            let err = restore_projects(&config, aside.path()).unwrap_err();
            let entry = existing.split('/').next().unwrap();
            assert!(
                matches!(&err, LangsError::RestoreConflict(path) if path.ends_with(entry)),
                "{err}"
            );
            assert_eq!(entry_names(projects_dir.path()), [entry]);
            assert_eq!(entry_names(aside.path()), ["course", "other"]);
            std::fs::remove_dir_all(projects_dir.path().join(entry)).unwrap();
        }
    }

    #[test]
    fn projects_cannot_be_set_aside_inside_the_projects_dir() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let projects_dir = tempfile::tempdir().unwrap();
        let config = config_with_projects_dir(projects_dir.path());
        file_to(projects_dir.path(), "course/file", "");

        let err = set_projects_aside(&config, &projects_dir.path().join("aside")).unwrap_err();
        assert!(
            matches!(err, LangsError::MovingProjectsDirIntoItself),
            "{err}"
        );
        assert_eq!(entry_names(projects_dir.path()), ["course"]);
        assert!(!projects_dir.path().join("aside").exists());

        let parent = projects_dir.path().parent().unwrap();
        let err = restore_projects(&config, parent).unwrap_err();
        assert!(
            matches!(err, LangsError::MovingProjectsDirIntoItself),
            "{err}"
        );
    }

    #[test]
    fn a_partial_move_of_dir_entries_is_rolled_back() {
        init();
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            file_to(from.path(), format!("{name}/file"), "");
        }
        // Renaming a directory onto a non-empty one fails.
        file_to(to.path(), "b/blocker", "");

        assert!(crate::move_all_dir_entries(from.path(), to.path()).is_err());
        assert_eq!(entry_names(from.path()), ["a", "b", "c"]);
        assert_eq!(entry_names(to.path()), ["b"]);
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

    #[test]
    fn moves_a_missing_projects_dir_by_repointing_the_config() {
        init();
        crate::test_util::ensure_isolated_locks_dir();

        let config_dir = tempfile::tempdir().unwrap();
        let location = config_dir.path().join("tmc_config.temp");
        let target_dir = tempfile::tempdir().unwrap();
        let tmc_config = TmcConfig {
            location: location.clone(),
            projects_dir: config_dir.path().join("unplugged"),
            table: Table::new(),
        };

        move_projects_dir(tmc_config, target_dir.path().to_path_buf()).unwrap();

        let saved = TmcConfig::load_from("client", location).unwrap();
        assert_eq!(saved.projects_dir, target_dir.path());
    }

    #[test]
    fn a_restore_creates_a_missing_projects_dir_with_an_existing_parent() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let parent = tempfile::tempdir().unwrap();
        let projects_dir = parent.path().join("projects");
        let config = config_with_projects_dir(&projects_dir);
        let aside = tempfile::tempdir().unwrap();
        file_to(aside.path(), "course/file", "contents");

        restore_projects(&config, aside.path()).unwrap();

        assert!(projects_dir.join("course/file").exists());
    }

    #[test]
    fn a_restore_still_fails_when_the_parent_is_missing_too() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let parent = tempfile::tempdir().unwrap();
        let unplugged = parent.path().join("drive/projects");
        let config = config_with_projects_dir(&unplugged);
        let aside = tempfile::tempdir().unwrap();
        file_to(aside.path(), "course/file", "contents");

        let err = restore_projects(&config, aside.path()).unwrap_err();

        assert!(matches!(err, LangsError::ProjectsDirUnreachable(path) if path == unplugged));
        assert!(!unplugged.exists());
    }

    #[test]
    fn the_move_check_refuses_like_the_move_and_moves_nothing() {
        init();
        crate::test_util::ensure_isolated_locks_dir();
        let projects_dir = tempfile::tempdir().unwrap();
        let config = config_with_projects_dir(projects_dir.path());
        file_to(projects_dir.path(), "course/file", "");

        let into_itself = projects_dir.path().join("inside");
        assert!(matches!(
            check_move_projects_dir(&config, &into_itself),
            Err(LangsError::MovingProjectsDirIntoItself)
        ));
        assert!(matches!(
            check_move_projects_dir(&config, projects_dir.path()),
            Err(LangsError::MovingProjectsDirToItself)
        ));
        let elsewhere = tempfile::tempdir().unwrap();
        check_move_projects_dir(&config, &elsewhere.path().join("new")).unwrap();
        assert!(!elsewhere.path().join("new").exists());
        assert!(projects_dir.path().join("course/file").exists());
    }

    #[test]
    fn refuses_to_move_the_projects_dir_into_itself() {
        init();
        crate::test_util::ensure_isolated_locks_dir();

        let config_dir = tempfile::tempdir().unwrap();
        let projects_dir = tempfile::tempdir().unwrap();
        let inside = projects_dir.path().join("inside");
        let tmc_config = TmcConfig {
            location: config_dir.path().join("tmc_config.temp"),
            projects_dir: projects_dir.path().to_path_buf(),
            table: Table::new(),
        };
        file_to(projects_dir.path(), "course/file", "");

        let result = move_projects_dir(tmc_config, inside);
        assert!(matches!(
            result,
            Err(LangsError::MovingProjectsDirIntoItself)
        ));
        assert!(projects_dir.path().join("course/file").exists());
    }
}
