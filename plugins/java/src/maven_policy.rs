//! Maven student file policy

use std::path::Path;
use tmc_langs_framework::{domain::TmcProjectYml, policy::StudentFilePolicy};

pub struct MavenStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for MavenStudentFilePolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    fn is_student_source_file(path: &Path) -> bool {
        path.starts_with("src/main")
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
