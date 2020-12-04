//! Contains the language policy for the plugin.

use std::path::Path;
use tmc_langs_framework::{domain::TmcProjectYml, policy::StudentFilePolicy};

pub struct MakeStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for MakeStudentFilePolicy {
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
        path.starts_with("src")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn is_student_source_file() {
        assert!(MakeStudentFilePolicy::is_student_source_file(Path::new(
            "src"
        )));
        assert!(MakeStudentFilePolicy::is_student_source_file(Path::new(
            "src/file"
        )));
        assert!(MakeStudentFilePolicy::is_student_source_file(Path::new(
            "src/dir/file"
        )));
    }

    #[test]
    fn is_not_student_source_file() {
        assert!(!MakeStudentFilePolicy::is_student_source_file(Path::new(
            "srcc"
        )));
        assert!(!MakeStudentFilePolicy::is_student_source_file(Path::new(
            "dir/src/file"
        )));
    }
}
