//! Student file policy for C#
use tmc_langs_framework::StudentFilePolicy;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
        let mut parent = path.parent();
        while let Some(next) = parent {
            if let Some(file_name) = next.file_name() {
                if file_name == OsString::from("bin") || file_name == OsString::from("obj") {
                    return true;
                }
            }
            parent = next.parent();
        }
        false
    }
}

impl StudentFilePolicy for CSharpStudentFilePolicy {
    // false for files in bin or obj directories, true for other files in src
    fn is_student_source_file(&self, path: &Path) -> bool {
        if self.is_child_of_binary_dir(path) {
            false
        } else {
            path.starts_with("src")
        }
    }

    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }
}
