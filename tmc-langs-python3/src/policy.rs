//! Contains the Python3 student file policy

use std::path::{Path, PathBuf};
use tmc_langs_framework::policy::StudentFilePolicy;

pub struct Python3StudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl Python3StudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for Python3StudentFilePolicy {
    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        path.starts_with("src")
    }
}
