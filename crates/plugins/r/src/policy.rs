//! Contains the R student file policy

use std::path::Path;
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

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

    fn is_non_extra_student_file(&self, path: &Path) -> bool {
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
    fn is_student_file() {
        init();

        let policy = RStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("R")));
        assert!(policy.is_student_file(Path::new("R/file")));
    }

    #[test]
    fn is_not_student_source_file() {
        init();

        let policy = RStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("dir/R")));
        assert!(!policy.is_student_file(Path::new("dir/R/file")));
    }
}
