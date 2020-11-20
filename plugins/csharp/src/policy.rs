//! Student file policy for C#

use std::path::{Path, PathBuf};
use tmc_langs_framework::StudentFilePolicy;

pub struct CSharpStudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl CSharpStudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }

    /// Goes up directories until a bin or obj directory is found
    fn is_child_of_binary_dir(&self, path: &Path) -> bool {
        for ancestor in path.ancestors() {
            if let Some(file_name) = ancestor.file_name() {
                if ancestor.is_dir() && (file_name == "bin" || file_name == "obj") {
                    return true;
                }
            }
        }
        false
    }
}

impl StudentFilePolicy for CSharpStudentFilePolicy {
    // false for files in bin or obj directories, true for other files in src
    fn is_student_source_file(&self, path: &Path) -> bool {
        path.starts_with("src") && !self.is_child_of_binary_dir(path)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }
}
