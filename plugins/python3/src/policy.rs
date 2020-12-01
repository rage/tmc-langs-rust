//! Contains the Python3 student file policy

use std::ffi::OsStr;
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
        // all non-pyc or __pycache__ files in src are student source files
        let in_src = path.starts_with("src");
        let is_cache_file = path.extension() == Some(OsStr::new("pyc"))
            || path
                .components()
                .any(|c| c.as_os_str() == OsStr::new("__pycache__"));

        // .py files in exercise root are student source files
        let is_in_project_root = match path.parent() {
            Some(s) => s.as_os_str().is_empty(),
            None => true,
        };
        let is_py_file = path.extension() == Some(OsStr::new("py"));

        // all files in non-tmc and non-test subdirectories are considered student files
        let is_in_exercise_subdir = path.starts_with("test") || path.starts_with("tmc");

        in_src && !is_cache_file
            || is_in_project_root && is_py_file
            || !is_in_exercise_subdir && !is_in_project_root && !is_cache_file
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn in_src_is_source_file() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(policy.is_student_source_file(Path::new("src/some_file.py")));
        assert!(policy.is_student_source_file(Path::new("src/some_dir/some_file.py")));
    }

    #[test]
    fn in_root_is_source_file() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(policy.is_student_source_file(Path::new("some_file.py")));
    }

    #[test]
    fn pycache_is_not_source_file() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(!policy.is_student_source_file(Path::new("__pycache__")));
        assert!(!policy.is_student_source_file(Path::new("__pycache__/cachefile")));
        assert!(!policy.is_student_source_file(Path::new("src/__pycache__")));
    }

    #[test]
    fn pyc_is_not_source_file() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(!policy.is_student_source_file(Path::new("some.pyc")));
        assert!(!policy.is_student_source_file(Path::new("src/other.pyc")));
    }

    #[test]
    fn subdirs_are_student_files() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(policy.is_student_source_file(Path::new("subdir/something")));
        assert!(policy.is_student_source_file(Path::new("another/mid/else")));
    }

    #[test]
    fn tmc_and_test_are_not_student_files() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(!policy.is_student_source_file(Path::new("test/something.py")));
        assert!(!policy.is_student_source_file(Path::new("tmc/mid/else.py")));
    }

    #[test]
    fn non_py_file_in_root_is_not_student_file() {
        let policy = Python3StudentFilePolicy::new(PathBuf::from(""));
        assert!(!policy.is_student_source_file(Path::new("test")));
        assert!(!policy.is_student_source_file(Path::new("root_file")));
    }
}
