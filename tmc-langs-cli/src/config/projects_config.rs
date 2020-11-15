use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct ProjectsConfig {
    pub courses: HashMap<String, CourseConfig>,
}

impl ProjectsConfig {
    pub fn load(path: &Path) -> Result<ProjectsConfig> {
        let mut course_configs = HashMap::new();
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CourseConfig {
    pub organization: String,
    pub course: String,
    pub exercises: HashMap<String, Exercise>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Exercise {
    pub checksum: usize,
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", "debug");
        }
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn serializes() {
        init();

        let mut exercises = HashMap::new();
        exercises.insert("ex 1".to_string(), Exercise { checksum: 1234 });
        let course_config = CourseConfig {
            organization: "org 1".to_string(),
            course: "course 1".to_string(),
            exercises,
        };
        let s = toml::to_string(&course_config).unwrap();
        log::debug!("\n{}", s);
        panic!();
    }

    #[test]
    fn deserializes() {
        let s = r#"
organization = "mooc org"
course = "python course"

[exercises.ex1]
checksum = 1234

[exercises."ex 2"]
checksum = 2345
"#;

        let _course_config: CourseConfig = toml::from_str(s).unwrap();
    }

    #[test]
    fn loads() {
        init();

        let mut pc = ProjectsConfig::load(Path::new("tests/data/config/projects_dir")).unwrap();
        assert_eq!(pc.courses.len(), 2);

        let mut cc = pc.courses.remove("course 1").unwrap();
        assert_eq!(cc.organization, "mooc");
        assert_eq!(cc.course, "python");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex1").unwrap();
        assert_eq!(ex.checksum, 1234);
        let ex = cc.exercises.remove("ex 2").unwrap();
        assert_eq!(ex.checksum, 2345);

        let mut cc = pc.courses.remove("course 2").unwrap();
        assert_eq!(cc.organization, "hy");
        assert_eq!(cc.course, "java");
        assert_eq!(cc.exercises.len(), 2);
        let ex = cc.exercises.remove("ex3").unwrap();
        assert_eq!(ex.checksum, 3456);
        let ex = cc.exercises.remove("ex 4").unwrap();
        assert_eq!(ex.checksum, 4567);
    }
}
