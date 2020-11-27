use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ProjectsConfig {
    // BTreeMap used so the exercises in the config file are ordered by key
    pub courses: BTreeMap<String, CourseConfig>,
}

impl ProjectsConfig {
    pub fn load(path: &Path) -> Result<ProjectsConfig> {
        let mut course_configs = BTreeMap::new();
        for file in fs::read_dir(path)
            .with_context(|| format!("Failed to read directory at {}", path.display()))?
        {
            let file =
                file.with_context(|| format!("Failed to read file in {}", path.display()))?;
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
                    path.display()
                );
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
    pub checksum: String,
}

#[cfg(test)]
mod test {
    use super::*;

    fn init_logging() {
        use simplelog::*;
        let _ = TestLogger::init(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .set_location_level(LevelFilter::Debug)
                .build(),
        );
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

    fn _dir_to(temp: impl AsRef<std::path::Path>, relative_path: impl AsRef<std::path::Path>) {
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
checksum = "abcd1234"

[exercises."ex 2"]
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
            "course 1/course_config.toml",
            r#"
course = "python"

[exercises.ex1]
checksum = "abcd1234"

[exercises."ex 2"]
checksum = "bcde2345"
"#,
        );
        file_to(
            &temp,
            "course 2/course_config.toml",
            r#"
course = "java"

[exercises.ex3]
checksum = "cdef3456"

[exercises."ex 4"]
checksum = "defg4567"
"#,
        );

        let mut pc = ProjectsConfig::load(temp.path()).unwrap();
        assert_eq!(pc.courses.len(), 2);

        let mut cc = pc.courses.remove("course 1").unwrap();
        assert_eq!(cc.course, "python");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex1").unwrap();
        assert_eq!(ex.checksum, "abcd1234");
        let ex = cc.exercises.remove("ex 2").unwrap();
        assert_eq!(ex.checksum, "bcde2345");

        let mut cc = pc.courses.remove("course 2").unwrap();
        assert_eq!(cc.course, "java");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex3").unwrap();
        assert_eq!(ex.checksum, "cdef3456");
        let ex = cc.exercises.remove("ex 4").unwrap();
        assert_eq!(ex.checksum, "defg4567");
    }
}
