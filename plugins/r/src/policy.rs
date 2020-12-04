//! Contains the R student file policy

use std::path::Path;
use tmc_langs_framework::{domain::TmcProjectYml, StudentFilePolicy};

pub struct RStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for RStudentFilePolicy {
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

        assert!(RStudentFilePolicy::is_student_source_file(Path::new("R")));
        assert!(RStudentFilePolicy::is_student_source_file(Path::new(
            "R/file"
        )));
    }

    #[test]
    fn is_not_student_source_file() {
        init();

        assert!(!RStudentFilePolicy::is_student_source_file(Path::new(
            "dir/R"
        )));
        assert!(!RStudentFilePolicy::is_student_source_file(Path::new(
            "dir/R/file"
        )));
    }
}
