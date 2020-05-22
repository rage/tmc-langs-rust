use std::path::{Path, PathBuf};
use tmc_langs_framework::policy::StudentFilePolicy;

pub struct MakeStudentFilePolicy {}

impl MakeStudentFilePolicy {
    pub fn new(path: PathBuf) -> Self {
        Self {}
    }
}

impl StudentFilePolicy for MakeStudentFilePolicy {
    fn get_config_file_parent_path(&self) -> &Path {
        todo!()
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        todo!()
    }
}
