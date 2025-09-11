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
use uuid::Uuid;
use walkdir::WalkDir;

/// A project directory is a directory which contains directories of backends
/// which contain directories of courses (which contain a `course_config.toml`).
const COURSE_CONFIG_FILE_NAME: &str = "course_config.toml";

#[derive(Debug)]
pub struct ProjectsConfig {
    // BTreeMap used so the exercises in the config file are ordered by key
    // slug => course
    pub tmc_courses: HashMap<String, TmcCourseConfig>,
    // instance_id => course
    pub mooc_courses: HashMap<Uuid, MoocCourseConfig>,
}

impl ProjectsConfig {
    pub fn load(projects_dir: &Path) -> Result<ProjectsConfig, LangsError> {
        let mut lock = Lock::dir(projects_dir, LockOptions::Read)?;
        let _guard = lock.lock()?;

        let mut tmc_course_configs = HashMap::<String, TmcCourseConfig>::new();
        let mut mooc_course_configs = HashMap::<Uuid, MoocCourseConfig>::new();

        // the projects dir has a separate directory for each TMC course, which are all expected to contain a `course_config.toml` file for TMC courses
        // MOOC courses are in a `mooc` subdirectory to prevent the course slugs (which are used as the directory names) from conflicting

        // process tmc courses first
        let mut unexpected_entries = Vec::new();
        let tmc_projects_dir = projects_dir.join("tmc");
        if tmc_projects_dir.exists() {
            for entry in WalkDir::new(tmc_projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                let file_name = entry.file_name();

                let course_config_path = entry.path().join(COURSE_CONFIG_FILE_NAME);
                if course_config_path.exists() {
                    let course_dir_name = file_name.to_str().ok_or_else(|| {
                        LangsError::FileError(FileError::NoFileName(entry.path().to_path_buf()))
                    })?;
                    let file = file_util::read_file_to_string(&course_config_path)?;
                    let course_config = deserialize::toml_from_str(&file)?;
                    tmc_course_configs.insert(course_dir_name.to_string(), course_config);
                } else {
                    unexpected_entries.push(entry);
                }
            }
        }

        // then mooc
        let mooc_projects_dir = projects_dir.join("mooc");
        if mooc_projects_dir.exists() {
            for entry in WalkDir::new(mooc_projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;

                let course_config_path = entry.path().join(COURSE_CONFIG_FILE_NAME);
                if course_config_path.exists() {
                    let file = file_util::read_file_to_string(&course_config_path)?;
                    let course_config = deserialize::toml_from_str::<MoocCourseConfig>(&file)?;
                    mooc_course_configs.insert(course_config.instance_id, course_config);
                } else {
                    unexpected_entries.push(entry);
                }
            }

            // no need to warn if the directory has no valid course directories at all
            if !(tmc_course_configs.is_empty() && mooc_course_configs.is_empty()) {
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
        }

        // there may also be old course directories in the projects dir directly,
        // as that is where they used to be stored
        // rather than move them, we'll just process them here to make things easier
        // users of langs can move them to the new dir if they like
        let mut unexpected_entries = Vec::new();
        let legacy_projects_dir = projects_dir;
        if legacy_projects_dir.exists() {
            for entry in WalkDir::new(legacy_projects_dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                let file_name = entry.file_name();

                if file_name == "tmc" || file_name == "mooc" {
                    // skip tmc and mooc dirs
                    continue;
                }

                // only tmc courses were ever stored this way
                let course_config_path = entry.path().join(COURSE_CONFIG_FILE_NAME);
                if course_config_path.exists() {
                    let course_dir_name = file_name.to_str().ok_or_else(|| {
                        LangsError::FileError(FileError::NoFileName(entry.path().to_path_buf()))
                    })?;
                    let file = file_util::read_file_to_string(&course_config_path)?;
                    let course_config = deserialize::toml_from_str(&file)?;
                    tmc_course_configs.insert(course_dir_name.to_string(), course_config);
                } else {
                    unexpected_entries.push(entry);
                }
            }
        }

        // maintenance: check that the exercises in the config actually exist on disk
        // if any are found that do not, update the course config file accordingly
        for (_, course_config) in tmc_course_configs.iter_mut() {
            let mut deleted_exercises = vec![];
            for exercise_name in course_config.exercises.keys() {
                let expected_dir = Self::get_tmc_exercise_download_target(
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
                course_config.save_to_projects_dir(projects_dir)?;
            }
        }
        for (_, course_config) in mooc_course_configs.iter_mut() {
            let mut deleted_exercises = vec![];
            for (exercise_id, exercise) in course_config.exercises.iter() {
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

    pub fn get_tmc_exercise_download_target(
        projects_dir: &Path,
        course_name: &str,
        exercise_name: &str,
    ) -> PathBuf {
        projects_dir
            .join("tmc")
            .join(course_name)
            .join(exercise_name)
    }

    pub fn get_mooc_exercise_download_target(
        projects_dir: &Path,
        instance_directory: &str,
        exercise_directory: &str,
    ) -> PathBuf {
        projects_dir
            .join("mooc")
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
        instance_id: Uuid,
        exercise_id: Uuid,
    ) -> Option<&ProjectsDirMoocExercise> {
        self.mooc_courses
            .get(&instance_id)
            .and_then(|c| c.exercises.get(&exercise_id))
    }

    pub fn get_all_tmc_exercises(&self) -> impl Iterator<Item = &ProjectsDirTmcExercise> {
        self.tmc_courses
            .iter()
            .flat_map(|c| &c.1.exercises)
            .map(|e| e.1)
    }

    pub fn get_all_mooc_exercises(&self) -> impl Iterator<Item = &ProjectsDirMoocExercise> {
        self.mooc_courses
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
                exercises: BTreeMap::new(),
            })
    }

    /// Note: does not save the config on initialization.
    pub fn get_or_init_mooc_course_config(
        &mut self,
        instance_id: Uuid,
        course_id: Uuid,
        course_name: String,
    ) -> &mut MoocCourseConfig {
        let existing_dirs = self
            .mooc_courses
            .values()
            .map(|mc| &mc.directory)
            .collect::<Vec<_>>();
        let kebab_name = simple_kebab_case(&course_name);
        let directory = if existing_dirs.contains(&&kebab_name) {
            // need to use another course name to avoid conflicts
            let mut dir = None;
            for i in 1.. {
                let proposal = format!("{kebab_name}-{i}");
                if !existing_dirs.contains(&&proposal) {
                    dir = Some(proposal);
                    break;
                }
            }
            dir.expect("unreachable")
        } else {
            kebab_name
        };
        self.mooc_courses
            .entry(instance_id)
            .or_insert(MoocCourseConfig {
                course: course_name,
                exercises: BTreeMap::new(),
                course_id,
                instance_id,
                directory,
            })
    }
}

fn simple_kebab_case(s: &str) -> String {
    s.to_lowercase().replace(" ", "-")
}

/// A course configuration file. Contains information of all of the exercises of this course in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct TmcCourseConfig {
    /// The course's name.
    pub course: String,
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
        let course_dir = projects_dir.join(&self.course);
        if !course_dir.exists() {
            file_util::create_dir_all(&course_dir)?;
        }
        let target = course_dir.join(COURSE_CONFIG_FILE_NAME);
        let s = toml::to_string_pretty(&self)?;
        file_util::write_to_file(s.as_bytes(), target)?;
        Ok(())
    }
}

/// A course configuration file. Contains information of all of the exercises of this course in the projects directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct MoocCourseConfig {
    pub course_id: Uuid,
    pub instance_id: Uuid,
    /// The course's name.
    pub course: String,
    pub directory: String,
    /// The course's exercises in a map with the exercise's id as the key.
    #[serde(default)]
    pub exercises: BTreeMap<Uuid, ProjectsDirMoocExercise>,
}

impl MoocCourseConfig {
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
}
