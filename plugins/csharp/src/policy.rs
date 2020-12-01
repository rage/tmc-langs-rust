//! Student file policy for the C# plugin.

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

    /// Goes up directories until a bin or obj directory is found, either indicating that the path is in a binary directory.
    fn is_child_of_binary_dir(path: &Path) -> bool {
        // checks each parent directory for bin or obj
        for ancestor in path.ancestors().skip(1) {
            if let Some(file_name) = ancestor.file_name() {
                if file_name == "bin" || file_name == "obj" {
                    return true;
                }
            }
        }
        false
    }
}

impl StudentFilePolicy for CSharpStudentFilePolicy {
    // false for files in bin or obj directories, true for other files in src.
    fn is_student_source_file(path: &Path) -> bool {
        path.starts_with("src") && !Self::is_child_of_binary_dir(path)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn file_in_binary_dir_is_not_student_file() {
        assert!(!CSharpStudentFilePolicy::is_student_source_file(Path::new(
            "src/bin/any/file"
        )));
        assert!(!CSharpStudentFilePolicy::is_student_source_file(Path::new(
            "obj/any/src/file"
        )));
    }

    #[test]
    fn file_in_src_is_student_file() {
        assert!(CSharpStudentFilePolicy::is_student_source_file(Path::new(
            "src/file"
        )));
        assert!(CSharpStudentFilePolicy::is_student_source_file(Path::new(
            "src/any/file"
        )));
    }
}
