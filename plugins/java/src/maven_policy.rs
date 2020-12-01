//! Maven student file policy

use std::path::{Path, PathBuf};
use tmc_langs_framework::policy::StudentFilePolicy;

pub struct MavenStudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl MavenStudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for MavenStudentFilePolicy {
    fn is_student_source_file(path: &Path) -> bool {
        path.starts_with("src/main")
    }

    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn is_student_source_file() {
        assert!(MavenStudentFilePolicy::is_student_source_file(Path::new(
            "src/main/file"
        )));
        assert!(MavenStudentFilePolicy::is_student_source_file(Path::new(
            "src/main/dir/file"
        )));
    }

    #[test]
    fn is_not_student_source_file() {
        assert!(!MavenStudentFilePolicy::is_student_source_file(Path::new(
            "file"
        )));
        assert!(!MavenStudentFilePolicy::is_student_source_file(Path::new(
            "dir/src/main/file"
        )));
        assert!(!MavenStudentFilePolicy::is_student_source_file(Path::new(
            "srca/main/file"
        )));
        assert!(!MavenStudentFilePolicy::is_student_source_file(Path::new(
            "src/mainc/file"
        )));
    }
}
