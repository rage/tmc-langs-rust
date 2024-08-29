//! Student file policy for the C# plugin.

use std::path::Path;
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct CSharpStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl CSharpStudentFilePolicy {
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
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    // false for .csproj files and files in bin or obj directories
    // true for files in src except for .csproj files
    fn is_non_extra_student_file(&self, path: &Path) -> bool {
        path.starts_with("src")
            // exclude files in bin
            && !Self::is_child_of_binary_dir(path)
            // exclude .csproj files
            && !path.extension().map(|ext| ext == "csproj").unwrap_or_default()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn file_in_binary_dir_is_not_student_file() {
        let policy = CSharpStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("src/bin/any/file")));
        assert!(!policy.is_student_file(Path::new("obj/any/src/file")));
    }

    #[test]
    fn file_in_src_is_student_file() {
        let policy = CSharpStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("src/file")));
        assert!(policy.is_student_file(Path::new("src/any/file")));
    }

    #[test]
    fn csproj_is_not_student_file() {
        let policy = CSharpStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("src/Project.csproj")));
    }
}
