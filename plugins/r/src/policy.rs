// Contains the R student file policy

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
