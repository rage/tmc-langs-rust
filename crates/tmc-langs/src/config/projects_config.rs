//! Structs for managing projects directories.

use crate::LangsError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, btree_map, hash_map::Entry},
    path::{Path, PathBuf},
};
use tmc_langs_framework::{is_valid_windows_file_name, is_windows_device_name};
use tmc_langs_util::{
    FileError, deserialize,
    file_util::{self, Lock, LockOptions},
};
use uuid::Uuid;
use walkdir::WalkDir;

/// A projects directory holds a directory per TMC course, as tmc-langs 0.39 laid
/// it out and older clients still read it, plus a `mooc` directory of mooc
/// courses. Each course directory contains a `course_config.toml`.
const COURSE_CONFIG_FILE_NAME: &str = "course_config.toml";
const MOOC_DIR_NAME: &str = "mooc";
/// tmc-langs 0.40–0.42 downloaded TMC exercises into `tmc/<course>/`; those are
/// still found there, but nothing new goes there except reserved-name courses.
const TMC_SPLIT_LAYOUT_DIR_NAME: &str = "tmc";

#[derive(Debug)]
pub struct ProjectsConfig {
    // BTreeMap used so the exercises in the config file are ordered by key
    // slug => course
    pub tmc_courses: HashMap<String, TmcCourseConfig>,
    // course_id => course
    pub mooc_courses: HashMap<Uuid, MoocCourseConfig>,
}

impl ProjectsConfig {
    pub fn load(projects_dir: &Path) -> Result<ProjectsConfig, LangsError> {
        let mut lock = Lock::dir(projects_dir, LockOptions::Read)?;
        let _guard = lock.lock()?;

        let mut tmc_course_configs = HashMap::<String, TmcCourseConfig>::new();
        let mut split_layout_tmc_course_configs = HashMap::<String, TmcCourseConfig>::new();
        let mut mooc_course_configs = HashMap::<Uuid, MoocCourseConfig>::new();

        let tmc_projects_dir = projects_dir.join(TMC_SPLIT_LAYOUT_DIR_NAME);
        if tmc_projects_dir.exists() {
            for entry in WalkDir::new(tmc_projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                // 0.40–0.42 course dirs here have no config; theirs is one level up
                if let Some((course_dir_name, course_config)) =
                    Self::read_tmc_course_config(&entry)?
                {
                    split_layout_tmc_course_configs.insert(course_dir_name, course_config);
                }
            }
        }

        let mut unexpected_entries = Vec::new();
        let mooc_projects_dir = projects_dir.join(MOOC_DIR_NAME);
        if mooc_projects_dir.exists() {
            for entry in WalkDir::new(&mooc_projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                if let Some(course_config) =
                    Self::read_mooc_course_config(&mooc_projects_dir, entry.path())?
                {
                    mooc_course_configs.insert(course_config.course_id, course_config);
                    continue;
                }
                // Versions that kept a '/' in course names wrote the config one level deeper.
                let mut nested_configs = Vec::new();
                if entry.file_type().is_dir() {
                    for nested in WalkDir::new(entry.path()).min_depth(1).max_depth(1) {
                        let nested = nested?;
                        if let Some(course_config) =
                            Self::read_mooc_course_config(&mooc_projects_dir, nested.path())?
                        {
                            nested_configs.push(course_config);
                        }
                    }
                }
                if nested_configs.is_empty() {
                    unexpected_entries.push(entry);
                }
                for course_config in nested_configs {
                    mooc_course_configs.insert(course_config.course_id, course_config);
                }
            }
        }

        if projects_dir.exists() {
            for entry in WalkDir::new(projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                if Self::is_reserved_dir_name(&entry.file_name().to_string_lossy()) {
                    continue;
                }
                if let Some((course_dir_name, course_config)) =
                    Self::read_tmc_course_config(&entry)?
                {
                    tmc_course_configs.insert(course_dir_name, course_config);
                }
            }
        }

        for (course_dir_name, split_layout_config) in split_layout_tmc_course_configs {
            match tmc_course_configs.entry(course_dir_name) {
                Entry::Vacant(vacant) => {
                    vacant.insert(split_layout_config);
                }
                Entry::Occupied(mut occupied) => {
                    occupied
                        .get_mut()
                        .merge_split_layout_config(projects_dir, split_layout_config);
                }
            }
        }

        // no need to warn if the directory has no valid course directories at all
        if !unexpected_entries.is_empty()
            && !(tmc_course_configs.is_empty() && mooc_course_configs.is_empty())
        {
            log::warn!(
                "Files or directories with no config files found \
                while loading projects from {}: [{}]",
                projects_dir.display(),
                unexpected_entries
                    .iter()
                    .filter_map(|ue| ue.path().as_os_str().to_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        // maintenance: check that the exercises in the config actually exist on disk
        // if any are found that do not, update the course config file accordingly
        for course_config in tmc_course_configs.values_mut() {
            let mut deleted_exercises = vec![];
            for exercise_name in course_config.exercises.keys() {
                if !Self::tmc_exercise_exists(projects_dir, &course_config.course, exercise_name) {
                    log::debug!(
                        "local exercise {} not found, deleting from config",
                        Self::get_tmc_exercise_download_target(
                            projects_dir,
                            &course_config.course,
                            exercise_name
                        )
                        .display()
                    );
                    deleted_exercises.push(exercise_name.clone());
                }
            }
            for deleted_exercise in &deleted_exercises {
                course_config
                    .exercises
                    .remove(deleted_exercise)
                    .expect("this should never fail");
            }
            if !deleted_exercises.is_empty() {
                course_config.save_to_projects_dir(projects_dir)?;
            }
        }
        for course_config in mooc_course_configs.values_mut() {
            let mut deleted_exercises = vec![];
            // Compared as files, so that names differing only in case match exactly where the
            // file system ignores case.
            let mut claimed_directories = HashSet::new();
            for (exercise_id, exercise) in course_config.exercises.iter() {
                if !is_safe_directory_name(&exercise.directory) {
                    log::warn!(
                        "forgetting local exercise {exercise_id}: its directory {:?} is not a \
                         plain directory name",
                        exercise.directory
                    );
                    deleted_exercises.push(*exercise_id);
                    continue;
                }
                let expected_dir = Self::get_mooc_exercise_download_target(
                    projects_dir,
                    &course_config.directory,
                    &exercise.directory,
                );
                if !expected_dir.exists() {
                    log::debug!(
                        "local exercise {} not found, deleting from config",
                        expected_dir.display()
                    );
                    deleted_exercises.push(*exercise_id);
                    continue;
                }
                if let Ok(directory) = same_file::Handle::from_path(&expected_dir)
                    && !claimed_directories.insert(directory)
                {
                    log::warn!(
                        "forgetting local exercise {exercise_id}: another exercise of the \
                         course is stored in its directory {:?}",
                        exercise.directory
                    );
                    deleted_exercises.push(*exercise_id);
                }
            }
            for deleted_exercise in &deleted_exercises {
                course_config
                    .exercises
                    .remove(deleted_exercise)
                    .expect("this should never fail");
            }
            if !deleted_exercises.is_empty() {
                course_config.save_to_projects_dir(projects_dir)?;
            }
        }

        Ok(Self {
            tmc_courses: tmc_course_configs,
            mooc_courses: mooc_course_configs,
        })
    }

    /// Exclusive lock for a download or update that reads, modifies and saves course configs.
    /// Keyed apart from the projects directory itself, which [`ProjectsConfig::load`] read-locks.
    pub fn lock_course_config_updates(projects_dir: &Path) -> Result<Lock, LangsError> {
        Ok(Lock::dir(
            projects_dir.join(".course-config-updates"),
            LockOptions::Write,
        )?)
    }

    /// Reads the TMC course config in a course directory, keyed by the directory's name.
    /// `None` if the entry has no config.
    fn read_tmc_course_config(
        entry: &walkdir::DirEntry,
    ) -> Result<Option<(String, TmcCourseConfig)>, LangsError> {
        let course_config_path = entry.path().join(COURSE_CONFIG_FILE_NAME);
        if !course_config_path.exists() {
            return Ok(None);
        }
        let course_dir_name = entry.file_name().to_str().ok_or_else(|| {
            LangsError::FileError(FileError::NoFileName(entry.path().to_path_buf()))
        })?;
        Ok(read_course_config(&course_config_path)
            .map(|course_config| (course_dir_name.to_string(), course_config)))
    }

    fn is_reserved_dir_name(name: &str) -> bool {
        name == TMC_SPLIT_LAYOUT_DIR_NAME || name == MOOC_DIR_NAME
    }

    /// The directory that holds a TMC course's config and its new downloads.
    fn tmc_course_dir(projects_dir: &Path, course_name: &str) -> PathBuf {
        // A course named after a backend directory would be read as that
        // directory, so it lives under `tmc/`, which every 0.40+ build reads.
        if Self::is_reserved_dir_name(course_name) {
            Self::split_layout_tmc_course_dir(projects_dir, course_name)
        } else {
            projects_dir.join(course_name)
        }
    }

    fn split_layout_tmc_course_dir(projects_dir: &Path, course_name: &str) -> PathBuf {
        projects_dir
            .join(TMC_SPLIT_LAYOUT_DIR_NAME)
            .join(course_name)
    }

    /// Where a TMC exercise that is not on disk yet is downloaded to.
    /// For one that may already be on disk, use [`Self::resolve_tmc_exercise_path`].
    pub fn get_tmc_exercise_download_target(
        projects_dir: &Path,
        course_name: &str,
        exercise_name: &str,
    ) -> PathBuf {
        Self::tmc_course_dir(projects_dir, course_name).join(exercise_name)
    }

    /// Where a TMC exercise is on disk: its download target, or the `tmc/`
    /// directory 0.40–0.42 downloaded it to. The download target if it is in neither.
    pub fn resolve_tmc_exercise_path(
        projects_dir: &Path,
        course_name: &str,
        exercise_name: &str,
    ) -> PathBuf {
        let target =
            Self::get_tmc_exercise_download_target(projects_dir, course_name, exercise_name);
        if target.exists() {
            return target;
        }
        let split_layout_path =
            Self::split_layout_tmc_course_dir(projects_dir, course_name).join(exercise_name);
        if split_layout_path.exists() {
            split_layout_path
        } else {
            target
        }
    }

    fn tmc_exercise_exists(projects_dir: &Path, course_name: &str, exercise_name: &str) -> bool {
        Self::resolve_tmc_exercise_path(projects_dir, course_name, exercise_name).exists()
    }

    /// The mooc course config in `course_dir`, if it has one. Its `directory` is taken from where
    /// it was found, never from the file, so it stays under `mooc_projects_dir`.
    fn read_mooc_course_config(
        mooc_projects_dir: &Path,
        course_dir: &Path,
    ) -> Result<Option<MoocCourseConfig>, LangsError> {
        let course_config_path = course_dir.join(COURSE_CONFIG_FILE_NAME);
        if !course_config_path.is_file() {
            return Ok(None);
        }
        let Some(mut course_config) = read_course_config::<MoocCourseConfig>(&course_config_path)
        else {
            return Ok(None);
        };
        let relative_dir = course_dir
            .strip_prefix(mooc_projects_dir)
            .expect("course dirs are walked from the mooc projects dir");
        course_config.directory = relative_dir
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        Ok(Some(course_config))
    }

    pub fn get_mooc_exercise_download_target(
        projects_dir: &Path,
        instance_directory: &str,
        exercise_directory: &str,
    ) -> PathBuf {
        projects_dir
            .join(MOOC_DIR_NAME)
            .join(instance_directory)
            .join(exercise_directory)
    }

    pub fn get_tmc_exercise(
        &self,
        course_name: &str,
        exercise_name: &str,
    ) -> Option<&ProjectsDirTmcExercise> {
        self.tmc_courses
            .get(course_name)
            .and_then(|c| c.exercises.get(exercise_name))
    }

    pub fn get_mooc_exercise(
        &self,
        course_id: Uuid,
        exercise_id: Uuid,
    ) -> Option<&ProjectsDirMoocExercise> {
        self.mooc_courses
            .get(&course_id)
            .and_then(|c| c.exercises.get(&exercise_id))
    }

    pub fn get_all_tmc_exercises(&self) -> impl Iterator<Item = &ProjectsDirTmcExercise> {
        self.tmc_courses
            .iter()
            .flat_map(|c| &c.1.exercises)
            .map(|e| e.1)
    }

    /// Note: does not save the config on initialization.
    pub fn get_or_init_tmc_course_config(&mut self, course_name: String) -> &mut TmcCourseConfig {
        self.tmc_courses
            .entry(course_name.clone())
            .or_insert(TmcCourseConfig {
                course: course_name,
                course_id: None,
                exercises: BTreeMap::new(),
            })
    }

    /// Note: does not save the config on initialization.
    pub fn get_or_init_mooc_course_config(
        &mut self,
        course_id: Uuid,
        course_name: String,
        is_exam: bool,
    ) -> &mut MoocCourseConfig {
        let directory = unique_directory_name(
            &course_name,
            self.mooc_courses.values().map(|mc| mc.directory.as_str()),
        );
        self.mooc_courses
            .entry(course_id)
            .or_insert(MoocCourseConfig {
                course: course_name,
                exercises: BTreeMap::new(),
                course_id,
                directory,
                is_exam,
            })
    }
}

/// Longest directory name derived from a server-supplied name, in characters; keeps exercise
/// paths well under Windows' 260-character limit.
const MAX_DIRECTORY_NAME_CHARS: usize = 64;

/// A directory name for a course or exercise name chosen on courses.mooc.fi: lower-cased letters,
/// digits, `-` and `_` only, so no name can escape the projects directory or be invalid on
/// Windows.
pub(crate) fn directory_name(name: &str) -> String {
    let mut directory = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            directory.push(c);
        } else if !directory.is_empty() && !directory.ends_with('-') {
            directory.push('-');
        }
    }
    let mut directory: String = directory
        .trim_end_matches('-')
        .chars()
        .take(MAX_DIRECTORY_NAME_CHARS)
        .collect();
    if directory.is_empty() {
        directory.push_str("unnamed");
    }
    if is_windows_device_name(&directory) {
        directory.push('_');
    }
    directory
}

/// Whether a stored directory name is a single, harmless path component. Names stored by
/// versions before [`directory_name`] may not be.
pub(crate) fn is_safe_directory_name(directory: &str) -> bool {
    if directory == directory_name(directory) {
        return true;
    }
    let is_one_component = !directory.is_empty()
        && directory != "."
        && directory != ".."
        && !directory.contains(['/', '\\', '\0']);
    // Older versions kept characters such as ':' that only Windows rejects, and those
    // directories exist and hold work everywhere else.
    is_one_component && (!cfg!(windows) || is_valid_windows_file_name(directory))
}

/// [`directory_name`] of `name`, suffixed to differ from every `taken` name. Compared without
/// case, as Windows and macOS file systems do.
pub(crate) fn unique_directory_name<'a>(
    name: &str,
    taken: impl IntoIterator<Item = &'a str>,
) -> String {
    let taken: HashSet<String> = taken.into_iter().map(str::to_lowercase).collect();
    let base = directory_name(name);
    if !taken.contains(&base) {
        return base;
    }
    (1..)
        .map(|i| format!("{base}-{i}"))
        .find(|proposal| !taken.contains(proposal))
        .expect("an unbounded range always yields a free suffix")
}

/// The course config at `path`, or `None` with a warning when it is unreadable, so one broken
/// file does not hide every other course.
fn read_course_config<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let parsed = file_util::read_file_to_string(path)
        .map_err(LangsError::from)
        .and_then(|file| Ok(deserialize::toml_from_str(&file)?));
    parsed
        .inspect_err(|err| log::warn!("Skipping {}: {err}", path.display()))
        .ok()
}

/// A course configuration file. Contains information of all of the exercises of this course in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct TmcCourseConfig {
    /// The course's name.
    pub course: String,
    /// The course's TMC id. Absent from a config written before ids were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub course_id: Option<u32>,
    /// The course's exercises in a map with the exercise's name as the key.
    #[serde(default)]
    pub exercises: BTreeMap<String, ProjectsDirTmcExercise>,
}

impl TmcCourseConfig {
    pub fn add_exercise(&mut self, exercise_name: String, id: u32, checksum: String) {
        let exercise = ProjectsDirTmcExercise { id, checksum };
        self.exercises.insert(exercise_name, exercise);
    }

    pub fn save_to_projects_dir(&self, projects_dir: &Path) -> Result<(), LangsError> {
        let course_dir = ProjectsConfig::tmc_course_dir(projects_dir, &self.course);
        if !course_dir.exists() {
            file_util::create_dir_all(&course_dir)?;
        }
        let target = course_dir.join(COURSE_CONFIG_FILE_NAME);
        let s = toml::to_string_pretty(&self)?;
        file_util::write_atomically(&target, &s)?;

        // `load` merges a `tmc/` copy back in, so a stale one would revert this save.
        let split_layout_target =
            ProjectsConfig::split_layout_tmc_course_dir(projects_dir, &self.course)
                .join(COURSE_CONFIG_FILE_NAME);
        if split_layout_target != target && split_layout_target.exists() {
            file_util::write_atomically(&split_layout_target, &s)?;
        }
        Ok(())
    }

    /// Adds the entries of a config found under `tmc/` for the same course.
    /// Where both list an exercise, the entry whose own directory holds it wins,
    /// this config's on a tie.
    fn merge_split_layout_config(
        &mut self,
        projects_dir: &Path,
        split_layout_config: TmcCourseConfig,
    ) {
        if self.course_id.is_none() {
            self.course_id = split_layout_config.course_id;
        }
        let own_dir = ProjectsConfig::tmc_course_dir(projects_dir, &self.course);
        let split_layout_dir =
            ProjectsConfig::split_layout_tmc_course_dir(projects_dir, &self.course);
        for (exercise_name, split_layout_exercise) in split_layout_config.exercises {
            match self.exercises.entry(exercise_name) {
                btree_map::Entry::Vacant(vacant) => {
                    vacant.insert(split_layout_exercise);
                }
                btree_map::Entry::Occupied(mut occupied) => {
                    if !own_dir.join(occupied.key()).exists()
                        && split_layout_dir.join(occupied.key()).exists()
                    {
                        occupied.insert(split_layout_exercise);
                    }
                }
            }
        }
    }
}

/// A course configuration file. Contains information of all of the exercises of this course in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct MoocCourseConfig {
    /// The exam's id when `is_exam`.
    /// The file also has an `instance_id`, always equal to this, for older CLIs; it is ignored.
    pub course_id: Uuid,
    /// The course's name.
    pub course: String,
    pub directory: String,
    /// Whether this holds an exam's exercises rather than a course's.
    #[serde(default)]
    pub is_exam: bool,
    /// The course's exercises in a map with the exercise's id as the key.
    #[serde(default)]
    pub exercises: BTreeMap<Uuid, ProjectsDirMoocExercise>,
}

impl MoocCourseConfig {
    /// The directory an exercise is or will be stored in: the stored one when it is safe, else
    /// one derived from `name` that no other exercise of the course, and none of `reserved`,
    /// uses.
    pub fn exercise_directory<'a>(
        &'a self,
        exercise_id: Uuid,
        name: &str,
        reserved: impl IntoIterator<Item = &'a String>,
    ) -> String {
        if let Some(existing) = self.exercises.get(&exercise_id)
            && is_safe_directory_name(&existing.directory)
        {
            return existing.directory.clone();
        }
        unique_directory_name(
            name,
            self.exercises
                .iter()
                .filter(|(id, _)| **id != exercise_id)
                .map(|(_, exercise)| exercise.directory.as_str())
                .chain(reserved.into_iter().map(String::as_str)),
        )
    }

    /// Inserts or replaces an exercise entry stored in `directory`, from
    /// [`Self::exercise_directory`].
    pub fn add_exercise(
        &mut self,
        exercise_id: Uuid,
        name: String,
        task_id: Uuid,
        checksum: String,
        directory: String,
    ) {
        self.exercises.insert(
            exercise_id,
            ProjectsDirMoocExercise {
                name,
                task_id,
                checksum,
                directory,
            },
        );
    }

    pub fn save_to_projects_dir(&self, projects_dir: &Path) -> Result<(), LangsError> {
        // The loader and `get_mooc_exercise_download_target` key off the sanitized
        // `directory`, not the raw course name; other paths orphan the config.
        let course_dir = projects_dir.join(MOOC_DIR_NAME).join(&self.directory);
        if !course_dir.exists() {
            file_util::create_dir_all(&course_dir)?;
        }
        let target = course_dir.join(COURSE_CONFIG_FILE_NAME);
        let mut table = toml::Table::try_from(self)?;
        // Earlier releases require it and fail to load the whole projects directory without it.
        table.insert("instance_id".into(), self.course_id.to_string().into());
        let s = toml::to_string_pretty(&table)?;
        file_util::write_atomically(&target, &s)?;
        Ok(())
    }
}

/// A TMC exercise in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectsDirTmcExercise {
    pub id: u32,
    pub checksum: String,
}

/// A MOOC exercise in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectsDirMoocExercise {
    pub name: String,
    pub task_id: Uuid,
    pub checksum: String,
    pub directory: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn init_logging() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            .with_module_level("j4rs", LevelFilter::Warn)
            .init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, contents.as_ref()).unwrap();
    }

    fn dir_to(temp: impl AsRef<std::path::Path>, relative_path: impl AsRef<std::path::Path>) {
        let target = temp.as_ref().join(relative_path);
        std::fs::create_dir_all(target).unwrap();
    }

    #[test]
    fn serializes() {
        init_logging();

        let mut exercises = BTreeMap::new();
        exercises.insert(
            "ex 1".to_string(),
            ProjectsDirTmcExercise {
                id: 4321,
                checksum: "abcd1234".to_string(),
            },
        );
        let course_config = TmcCourseConfig {
            course: "course 1".to_string(),
            course_id: None,
            exercises,
        };
        let s = toml::to_string(&course_config).unwrap();
        assert_eq!(
            s,
            r#"course = "course 1"

[exercises."ex 1"]
id = 4321
checksum = "abcd1234"
"#
        )
    }

    #[test]
    fn deserializes() {
        init_logging();

        let s = r#"
course = "python course"

[exercises.ex1]
id = 4321
checksum = "abcd1234"

[exercises."ex 2"]
id = 5432
checksum = "bcde2345"
"#;

        let _course_config: TmcCourseConfig = deserialize::toml_from_str(s).unwrap();
    }

    #[test]
    fn course_id_round_trips() {
        let course_config = TmcCourseConfig {
            course: "course 1".to_string(),
            course_id: Some(17),
            exercises: BTreeMap::new(),
        };
        let s = toml::to_string(&course_config).unwrap();
        assert_eq!(s, "course = \"course 1\"\ncourse_id = 17\n\n[exercises]\n");
        let course_config: TmcCourseConfig = deserialize::toml_from_str(&s).unwrap();
        assert_eq!(course_config.course_id, Some(17));
    }

    #[test]
    fn deserializes_a_config_without_course_id() {
        let course_config: TmcCourseConfig =
            deserialize::toml_from_str("course = \"python course\"\n").unwrap();
        assert_eq!(course_config.course_id, None);
    }

    #[test]
    fn loads() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        file_to(
            &temp,
            "python/course_config.toml",
            r#"
course = "python"

[exercises.ex1]
id = 4321
checksum = "abcd1234"

[exercises."ex 2"]
id = 5432
checksum = "bcde2345"
"#,
        );
        dir_to(&temp, "tmc/python/ex1");
        dir_to(&temp, "tmc/python/ex 2");
        file_to(
            &temp,
            "java/course_config.toml",
            r#"
course = "java"

[exercises.ex3]
id = 6543
checksum = "cdef3456"

[exercises."ex 4"]
id = 7654
checksum = "defg4567"
"#,
        );
        dir_to(&temp, "tmc/java/ex3");
        dir_to(&temp, "tmc/java/ex 4");

        let mut pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.tmc_courses.len(), 2);

        let mut cc = pc.tmc_courses.remove("python").unwrap();
        assert_eq!(cc.course, "python");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex1").unwrap();
        assert_eq!(ex.id, 4321);
        assert_eq!(ex.checksum, "abcd1234");
        let ex = cc.exercises.remove("ex 2").unwrap();
        assert_eq!(ex.id, 5432);
        assert_eq!(ex.checksum, "bcde2345");

        let mut cc = pc.tmc_courses.remove("java").unwrap();
        assert_eq!(cc.course, "java");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex3").unwrap();
        assert_eq!(ex.id, 6543);
        assert_eq!(ex.checksum, "cdef3456");
        let ex = cc.exercises.remove("ex 4").unwrap();
        assert_eq!(ex.id, 7654);
        assert_eq!(ex.checksum, "defg4567");
    }

    fn tmc_config(course: &str, exercises: &[(&str, u32, &str)]) -> String {
        let mut s = format!("course = '{course}'\n");
        for (name, id, checksum) in exercises {
            s.push_str(&format!(
                "[exercises.'{name}']\nid = {id}\nchecksum = '{checksum}'\n"
            ));
        }
        s
    }

    fn read_config(path: impl AsRef<Path>) -> TmcCourseConfig {
        deserialize::toml_from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_legacy_layout_survives_loading() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        file_to(
            &temp,
            "course1/course_config.toml",
            tmc_config("course1", &[("ex1", 1, "a")]),
        );
        dir_to(&temp, "course1/ex1");

        let pc = ProjectsConfig::load(temp.path()).unwrap();

        assert_eq!(pc.tmc_courses["course1"].exercises["ex1"].id, 1);
        assert_eq!(
            read_config(temp.path().join("course1/course_config.toml"))
                .exercises
                .len(),
            1
        );
        assert_eq!(
            ProjectsConfig::resolve_tmc_exercise_path(temp.path(), "course1", "ex1"),
            temp.path().join("course1/ex1")
        );
    }

    #[test]
    fn new_tmc_downloads_target_the_legacy_layout() {
        let projects_dir = Path::new("projects");
        assert_eq!(
            ProjectsConfig::get_tmc_exercise_download_target(projects_dir, "course", "ex"),
            projects_dir.join("course").join("ex")
        );
        assert_eq!(
            ProjectsConfig::resolve_tmc_exercise_path(projects_dir, "course", "ex"),
            projects_dir.join("course").join("ex")
        );
    }

    #[test]
    fn reserved_course_names_are_kept_under_tmc() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        for course in ["tmc", "mooc"] {
            assert_eq!(
                ProjectsConfig::get_tmc_exercise_download_target(temp.path(), course, "ex"),
                temp.path().join("tmc").join(course).join("ex")
            );
            let mut cc = TmcCourseConfig {
                course: course.to_string(),
                course_id: None,
                exercises: BTreeMap::new(),
            };
            cc.add_exercise("ex".to_string(), 1, "a".to_string());
            cc.save_to_projects_dir(temp.path()).unwrap();
            dir_to(&temp, format!("tmc/{course}/ex"));
        }

        let pc = ProjectsConfig::load(temp.path()).unwrap();

        assert_eq!(pc.tmc_courses.len(), 2);
        assert!(pc.tmc_courses["tmc"].exercises.contains_key("ex"));
        assert!(pc.tmc_courses["mooc"].exercises.contains_key("ex"));
        assert!(pc.mooc_courses.is_empty());
    }

    #[test]
    fn a_split_layout_config_loads_and_resolves() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        file_to(
            &temp,
            "tmc/course/course_config.toml",
            tmc_config("course", &[("ex", 1, "a")]),
        );
        dir_to(&temp, "tmc/course/ex");

        let pc = ProjectsConfig::load(temp.path()).unwrap();

        assert_eq!(pc.tmc_courses["course"].exercises["ex"].id, 1);
        assert_eq!(
            ProjectsConfig::resolve_tmc_exercise_path(temp.path(), "course", "ex"),
            temp.path().join("tmc/course/ex")
        );
    }

    #[test]
    fn the_legacy_location_wins_when_an_exercise_is_in_both_layouts() {
        let temp = tempfile::TempDir::new().unwrap();
        dir_to(&temp, "course/ex");
        dir_to(&temp, "tmc/course/ex");

        assert_eq!(
            ProjectsConfig::resolve_tmc_exercise_path(temp.path(), "course", "ex"),
            temp.path().join("course/ex")
        );
    }

    #[test]
    fn configs_in_both_layouts_merge_and_save_without_losing_entries() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        let legacy_config = temp.path().join("course/course_config.toml");
        file_to(
            &temp,
            "course/course_config.toml",
            tmc_config(
                "course",
                &[
                    ("legacy only", 1, "a"),
                    ("in both, legacy dir", 2, "legacy"),
                    ("in both, split dir", 4, "legacy"),
                ],
            ),
        );
        file_to(
            &temp,
            "tmc/course/course_config.toml",
            tmc_config(
                "course",
                &[
                    ("split only", 3, "c"),
                    ("in both, legacy dir", 2, "split"),
                    ("in both, split dir", 4, "split"),
                ],
            )
            .replacen('\n', "\ncourse_id = 7\n", 1),
        );
        dir_to(&temp, "course/legacy only");
        dir_to(&temp, "course/in both, legacy dir");
        dir_to(&temp, "tmc/course/split only");
        dir_to(&temp, "tmc/course/in both, split dir");

        let mut pc = ProjectsConfig::load(temp.path()).unwrap();

        let cc = pc.tmc_courses.get_mut("course").unwrap();
        assert_eq!(cc.course_id, Some(7));
        let checksums = cc
            .exercises
            .iter()
            .map(|(name, e)| (name.as_str(), e.checksum.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            checksums,
            [
                ("in both, legacy dir", "legacy"),
                ("in both, split dir", "split"),
                ("legacy only", "a"),
                ("split only", "c"),
            ]
        );

        cc.add_exercise("in both, split dir".to_string(), 4, "updated".to_string());
        cc.save_to_projects_dir(temp.path()).unwrap();
        for saved in [
            read_config(&legacy_config),
            read_config(temp.path().join("tmc/course/course_config.toml")),
        ] {
            assert_eq!(saved.exercises.len(), 4);
            assert_eq!(saved.exercises["in both, split dir"].checksum, "updated");
        }
        let reloaded = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(
            reloaded.tmc_courses["course"].exercises["in both, split dir"].checksum,
            "updated"
        );
    }

    #[test]
    fn maintenance_removes_only_exercises_missing_from_both_layouts() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        file_to(
            &temp,
            "course/course_config.toml",
            tmc_config(
                "course",
                &[("legacy", 1, "a"), ("split", 2, "b"), ("missing", 3, "c")],
            ),
        );
        dir_to(&temp, "course/legacy");
        dir_to(&temp, "tmc/course/split");

        let pc = ProjectsConfig::load(temp.path()).unwrap();

        let names = |cc: &TmcCourseConfig| cc.exercises.keys().cloned().collect::<Vec<_>>();
        assert_eq!(names(&pc.tmc_courses["course"]), ["legacy", "split"]);
        assert_eq!(
            names(&read_config(temp.path().join("course/course_config.toml"))),
            ["legacy", "split"]
        );
        assert!(!temp.path().join("tmc/course/course_config.toml").exists());
    }

    #[test]
    fn directory_names_cannot_escape_or_break_on_windows() {
        for (name, expected) in [
            ("Part 1: Hello world", "part-1-hello-world"),
            ("../../../..", "unnamed"),
            ("/etc/passwd", "etc-passwd"),
            ("C:\\Windows", "c-windows"),
            ("Ohjelmointi: osa 1/2", "ohjelmointi-osa-1-2"),
            ("what? *now* <a|b> \"q\"", "what-now-a-b-q"),
            ("trailing dot.", "trailing-dot"),
            ("Con", "con_"),
            ("com1", "com1_"),
            ("Äidinkieli", "äidinkieli"),
            ("snake_case", "snake_case"),
        ] {
            assert_eq!(directory_name(name), expected, "{name}");
            assert!(is_safe_directory_name(&directory_name(name)));
        }
        assert_eq!(directory_name(&"x".repeat(200)).chars().count(), 64);
    }

    #[test]
    fn stored_directory_names_with_path_syntax_are_unsafe() {
        for unsafe_name in ["..", ".", "", "a/b", "a\\b"] {
            assert!(!is_safe_directory_name(unsafe_name), "{unsafe_name}");
        }
        for safe_name in ["exercise-1", "My Exercise", "ex.1"] {
            assert!(is_safe_directory_name(safe_name), "{safe_name}");
        }
        for windows_invalid_name in ["c:", "nul", "aux.txt", "dot.", "part-1:-hello-world"] {
            assert_eq!(
                is_safe_directory_name(windows_invalid_name),
                !cfg!(windows),
                "{windows_invalid_name}"
            );
        }
    }

    #[test]
    fn names_equal_up_to_case_or_spacing_get_distinct_directories() {
        let mut course = MoocCourseConfig {
            course_id: Uuid::new_v4(),
            course: "c".to_string(),
            directory: "c".to_string(),
            is_exam: false,
            exercises: BTreeMap::new(),
        };
        let mut directories = Vec::new();
        for name in ["U15 Dup", "u15 dup", "u15-dup"] {
            let id = Uuid::new_v4();
            let directory = course.exercise_directory(id, name, []);
            course.add_exercise(id, name.to_string(), id, String::new(), directory.clone());
            directories.push(directory);
        }
        assert_eq!(directories, ["u15-dup", "u15-dup-1", "u15-dup-2"]);
    }

    #[test]
    fn course_directory_comes_from_where_the_config_is() {
        let temp = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        file_to(
            &temp,
            "mooc/Ohjelmointi osa 1/2/course_config.toml",
            format!(
                "course_id = \"{id}\"\ninstance_id = \"{id}\"\ncourse = \"c\"\n\
                 directory = \"../../elsewhere\"\n"
            ),
        );
        let pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.mooc_courses[&id].directory, "Ohjelmointi osa 1/2");
    }

    #[test]
    fn exercises_sharing_a_directory_keep_only_one_entry() {
        init_logging();
        let temp = tempfile::TempDir::new().unwrap();
        let course_id = Uuid::new_v4();
        let exercise = |directory: &str| ProjectsDirMoocExercise {
            name: directory.to_string(),
            task_id: Uuid::new_v4(),
            checksum: "abcd".to_string(),
            directory: directory.to_string(),
        };
        let exercises = BTreeMap::from([
            (Uuid::new_v4(), exercise("ex")),
            (Uuid::new_v4(), exercise("ex")),
            (Uuid::new_v4(), exercise("missing")),
            (Uuid::new_v4(), exercise("other")),
        ]);
        MoocCourseConfig {
            course_id,
            course: "course".to_string(),
            directory: "course".to_string(),
            is_exam: false,
            exercises,
        }
        .save_to_projects_dir(temp.path())
        .unwrap();
        for directory in ["ex", "other"] {
            dir_to(&temp, format!("mooc/course/{directory}"));
        }

        let pc = ProjectsConfig::load(temp.path()).unwrap();
        let mut directories = pc.mooc_courses[&course_id]
            .exercises
            .values()
            .map(|exercise| exercise.directory.clone())
            .collect::<Vec<_>>();
        directories.sort();
        assert_eq!(directories, ["ex", "other"]);
    }

    #[test]
    fn mooc_config_save_load_round_trip() {
        // Save and load must agree on the `mooc/<directory>/` layout.
        init_logging();

        let temp = tempfile::TempDir::new().unwrap();

        let course_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let exercise_id = Uuid::new_v4();

        let mut exercises = BTreeMap::new();
        exercises.insert(
            exercise_id,
            ProjectsDirMoocExercise {
                name: "Exercise 1".to_string(),
                task_id,
                checksum: "abcd1234".to_string(),
                directory: "exercise-1".to_string(),
            },
        );
        let course_config = MoocCourseConfig {
            course_id,
            course: "My MOOC Course".to_string(),
            directory: "my-mooc-course".to_string(),
            is_exam: false,
            exercises,
        };
        course_config.save_to_projects_dir(temp.path()).unwrap();

        let expected_config_path = temp
            .path()
            .join("mooc")
            .join("my-mooc-course")
            .join(COURSE_CONFIG_FILE_NAME);
        assert!(
            expected_config_path.exists(),
            "config should be saved under mooc/<directory>/, was not found at {}",
            expected_config_path.display()
        );
        let saved: toml::Table =
            toml::from_str(&std::fs::read_to_string(&expected_config_path).unwrap()).unwrap();
        assert_eq!(saved["instance_id"].as_str(), Some(&*course_id.to_string()));

        // create the exercise dir so the load maintenance pass doesn't prune it
        dir_to(&temp, "mooc/my-mooc-course/exercise-1");

        let pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.mooc_courses.len(), 1);
        let cc = pc.mooc_courses.get(&course_id).unwrap();
        assert_eq!(cc.course, "My MOOC Course");
        assert_eq!(cc.directory, "my-mooc-course");
        assert_eq!(cc.course_id, course_id);
        assert_eq!(cc.exercises.len(), 1);
        let ex = cc.exercises.get(&exercise_id).unwrap();
        assert_eq!(ex.task_id, task_id);
        assert_eq!(ex.checksum, "abcd1234");
        assert_eq!(ex.directory, "exercise-1");
    }

    #[test]
    fn an_unparsable_course_config_is_skipped_not_fatal() {
        init_logging();
        crate::test_util::ensure_isolated_locks_dir();

        let temp = tempfile::TempDir::new().unwrap();
        file_to(
            &temp,
            "broken/course_config.toml",
            "course = \"unterminated",
        );
        file_to(&temp, "mooc/broken/course_config.toml", "not = [valid");
        file_to(
            &temp,
            "python/course_config.toml",
            "course = \"python\"\n[exercises.ex1]\nid = 1\nchecksum = \"a\"\n",
        );
        dir_to(&temp, "python/ex1");

        let pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.tmc_courses.len(), 1);
        assert!(pc.tmc_courses.contains_key("python"));
        assert!(pc.mooc_courses.is_empty());
    }

    #[test]
    fn saving_a_course_config_replaces_it_whole_and_leaves_no_temp_files() {
        init_logging();

        let temp = tempfile::TempDir::new().unwrap();
        let mut course_config = TmcCourseConfig {
            course: "python".to_string(),
            course_id: None,
            exercises: BTreeMap::new(),
        };
        course_config.save_to_projects_dir(temp.path()).unwrap();
        course_config.add_exercise("ex1".to_string(), 1, "a".to_string());
        course_config.save_to_projects_dir(temp.path()).unwrap();

        let course_dir = temp.path().join("python");
        let names = std::fs::read_dir(&course_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, [std::ffi::OsString::from(COURSE_CONFIG_FILE_NAME)]);
        let saved: TmcCourseConfig = deserialize::toml_from_str(
            &std::fs::read_to_string(course_dir.join(COURSE_CONFIG_FILE_NAME)).unwrap(),
        )
        .unwrap();
        assert!(saved.exercises.contains_key("ex1"));
    }
}
