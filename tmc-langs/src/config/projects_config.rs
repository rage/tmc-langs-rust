use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tmc_langs_util::file_util;

#[derive(Debug)]
pub struct ProjectsConfig {
    // BTreeMap used so the exercises in the config file are ordered by key
    pub courses: BTreeMap<String, CourseConfig>,
}

impl ProjectsConfig {
    pub fn load(projects_dir: &Path) -> Result<ProjectsConfig> {
        file_util::lock!(projects_dir);

        let mut course_configs = BTreeMap::new();
        for file in fs::read_dir(projects_dir)
            .with_context(|| format!("Failed to read directory at {}", projects_dir.display()))?
        {
            let file =
                file.with_context(|| format!("Failed to read file in {}", projects_dir.display()))?;
            let course_config_path = file.path().join("course_config.toml");
            if course_config_path.exists() {
                let file_name = file.file_name();
                let course_dir_name = file_name.to_str().with_context(|| {
                    format!(
                        "Course directory name was not valid utf-8: {}",
                        file.file_name().to_string_lossy()
                    )
                })?;

                let bytes = fs::read(course_config_path)?;
                let course_config: CourseConfig = toml::from_slice(&bytes)?;

                course_configs.insert(course_dir_name.to_string(), course_config);
            } else {
                log::warn!(
                    "File or directory {} with no config file found while loading projects from {}",
                    file.path().display(),
                    projects_dir.display()
                );
            }
        }

        // maintenance: check that the exercises in the config actually exist on disk
        // if any are found that do not, update the course config file accordingly
        for (_, course_config) in course_configs.iter_mut() {
            let mut deleted_exercises = vec![];
            for exercise_name in course_config.exercises.keys() {
                let expected_dir = Self::get_exercise_download_target(
                    projects_dir,
                    &course_config.course,
                    &exercise_name,
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
                course_config.exercises.remove(deleted_exercise).unwrap(); // cannot fail
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CourseConfig {
    pub course: String,
    pub exercises: BTreeMap<String, Exercise>,
}

impl CourseConfig {
    pub fn save_to_projects_dir(&self, projects_dir: &Path) -> Result<()> {
        file_util::lock!(projects_dir);

        let course_dir = projects_dir.join(&self.course);
        if !course_dir.exists() {
            fs::create_dir_all(&course_dir).with_context(|| {
                format!(
                    "Failed to create course directory at {}",
                    course_dir.display()
                )
            })?;
        }
        let target = course_dir.join("course_config.toml");
        let s = toml::to_string_pretty(&self)?;
        fs::write(&target, s.as_bytes())
            .with_context(|| format!("Failed to write course config to {}", target.display()))?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Exercise {
    pub id: usize,
    pub checksum: String,
}

#[cfg(test)]
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
            Exercise {
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

        let _course_config: CourseConfig = toml::from_str(s).unwrap();
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
