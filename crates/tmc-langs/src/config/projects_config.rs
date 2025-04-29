//! Structs for managing projects directories.

use crate::LangsError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};
use tmc_langs_util::{
    FileError, deserialize,
    file_util::{self, Lock, LockOptions},
};
use walkdir::WalkDir;

/// A project directory is a directory which contains directories of courses (which contain a `course_config.toml`).
const COURSE_CONFIG_FILE_NAME: &str = "course_config.toml";

#[derive(Debug)]
pub struct ProjectsConfig {
    // BTreeMap used so the exercises in the config file are ordered by key
    pub courses: HashMap<String, CourseConfig>,
}

impl ProjectsConfig {
    pub fn load(projects_dir: &Path) -> Result<ProjectsConfig, LangsError> {
        let mut lock = Lock::dir(projects_dir, LockOptions::Read)?;
        let _guard = lock.lock()?;
        let mut course_configs = HashMap::new();

        let mut unexpected_entries = Vec::new();
        for entry in WalkDir::new(projects_dir).min_depth(1).max_depth(1) {
            let entry = entry?;
            let course_config_path = entry.path().join(COURSE_CONFIG_FILE_NAME);
            if course_config_path.exists() {
                let file_name = entry.file_name();
                let course_dir_name = file_name.to_str().ok_or_else(|| {
                    LangsError::FileError(FileError::NoFileName(entry.path().to_path_buf()))
                })?;
                let file = file_util::read_file_to_string(&course_config_path)?;
                let course_config: CourseConfig = deserialize::toml_from_str(&file)?;

                course_configs.insert(course_dir_name.to_string(), course_config);
            } else {
                unexpected_entries.push(entry);
            }
        }

        // no need to warn if the directory has no valid course directories at all
        if !course_configs.is_empty() {
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
        for (_, course_config) in course_configs.iter_mut() {
            let mut deleted_exercises = vec![];
            for exercise_name in course_config.exercises.keys() {
                let expected_dir = Self::get_exercise_download_target(
                    projects_dir,
                    &course_config.course,
                    exercise_name,
                );
                if !expected_dir.exists() {
                    log::debug!(
                        "local exercise {} not found, deleting from config",
                        expected_dir.display()
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
                // if any exercises were deleted, save the course config
                course_config.save_to_projects_dir(projects_dir)?;
            }
        }
        Ok(Self {
            courses: course_configs,
        })
    }

    pub fn get_exercise_download_target(
        projects_dir: &Path,
        course_name: &str,
        exercise_name: &str,
    ) -> PathBuf {
        projects_dir.join(course_name).join(exercise_name)
    }

    pub fn get_exercise(
        &self,
        course_name: &str,
        exercise_name: &str,
    ) -> Option<&ProjectsDirExercise> {
        self.courses
            .get(course_name)
            .and_then(|c| c.exercises.get(exercise_name))
    }

    pub fn get_all_exercises(&self) -> impl Iterator<Item = &ProjectsDirExercise> {
        self.courses
            .iter()
            .flat_map(|c| &c.1.exercises)
            .map(|e| e.1)
    }

    /// Note: does not save the config on initialization.
    pub fn get_or_init_course_config(&mut self, course_name: String) -> &mut CourseConfig {
        self.courses
            .entry(course_name.clone())
            .or_insert(CourseConfig {
                course: course_name,
                exercises: BTreeMap::new(),
            })
    }
}

/// A course configuration file. Contains information of all of the exercises of this course in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct CourseConfig {
    /// The course's name.
    pub course: String,
    /// The course's exercises in a map with the exercise's name as the key.
    pub exercises: BTreeMap<String, ProjectsDirExercise>,
}

impl CourseConfig {
    pub fn save_to_projects_dir(&self, projects_dir: &Path) -> Result<(), LangsError> {
        let course_dir = projects_dir.join(&self.course);
        if !course_dir.exists() {
            file_util::create_dir_all(&course_dir)?;
        }
        let target = course_dir.join(COURSE_CONFIG_FILE_NAME);
        let s = toml::to_string_pretty(&self)?;
        file_util::write_to_file(s.as_bytes(), target)?;
        Ok(())
    }

    pub fn add_exercise(&mut self, exercise_name: String, id: u32, checksum: String) {
        let exercise = ProjectsDirExercise { id, checksum };
        self.exercises.insert(exercise_name, exercise);
    }
}

/// An exercise in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectsDirExercise {
    pub id: u32,
    pub checksum: String,
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
            ProjectsDirExercise {
                id: 4321,
                checksum: "abcd1234".to_string(),
            },
        );
        let course_config = CourseConfig {
            course: "course 1".to_string(),
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

        let _course_config: CourseConfig = deserialize::toml_from_str(s).unwrap();
    }

    #[test]
    fn loads() {
        init_logging();

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
        dir_to(&temp, "python/ex1");
        dir_to(&temp, "python/ex 2");
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
        dir_to(&temp, "java/ex3");
        dir_to(&temp, "java/ex 4");

        let mut pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.courses.len(), 2);

        let mut cc = pc.courses.remove("python").unwrap();
        assert_eq!(cc.course, "python");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex1").unwrap();
        assert_eq!(ex.id, 4321);
        assert_eq!(ex.checksum, "abcd1234");
        let ex = cc.exercises.remove("ex 2").unwrap();
        assert_eq!(ex.id, 5432);
        assert_eq!(ex.checksum, "bcde2345");

        let mut cc = pc.courses.remove("java").unwrap();
        assert_eq!(cc.course, "java");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex3").unwrap();
        assert_eq!(ex.id, 6543);
        assert_eq!(ex.checksum, "cdef3456");
        let ex = cc.exercises.remove("ex 4").unwrap();
        assert_eq!(ex.id, 7654);
        assert_eq!(ex.checksum, "defg4567");
    }
}
