//! Contains the R student file policy

use std::path::{Path, PathBuf};
use tmc_langs_framework::StudentFilePolicy;

pub struct RStudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl RStudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for RStudentFilePolicy {
    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        path.starts_with("R")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    #[test]
    fn is_student_source_file() {
        init();

        let policy = RStudentFilePolicy::new(PathBuf::from(""));
        assert!(policy.is_student_source_file(Path::new("R")));
        assert!(policy.is_student_source_file(Path::new("R/file")));
    }

    #[test]
    fn is_not_student_source_file() {
        init();

        let policy = RStudentFilePolicy::new(PathBuf::from(""));
        assert!(!policy.is_student_source_file(Path::new("dir/R")));
        assert!(!policy.is_student_source_file(Path::new("dir/R/file")));
    }
}
