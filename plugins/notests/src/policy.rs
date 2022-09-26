//! The student file policy for no-tests projects.

use std::path::Path;
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

/// The no-tests policy considers all files to be student source files by default.
pub struct NoTestsStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for NoTestsStudentFilePolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    fn is_student_source_file(&self, _file_path: &Path) -> bool {
        true
    }
}
