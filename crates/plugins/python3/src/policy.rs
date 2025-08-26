//! Contains the Python3 student file policy

use std::{ffi::OsStr, path::Path};
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct Python3StudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for Python3StudentFilePolicy {
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
        // no files in tmc, test and venv subdirectories are considered student files
        let is_cache_file = path.extension() == Some(OsStr::new("pyc"))
            || path
                .components()
                .any(|c| c.as_os_str() == OsStr::new("__pycache__"));
        let is_in_exercise_subdir = path.starts_with("test") || path.starts_with("tmc");
        let is_in_venv = path.starts_with(".venv") || path.starts_with("venv");
        if is_cache_file || is_in_exercise_subdir || is_in_venv {
            return false;
        }

        // all non-pyc or __pycache__ files in src are student source files
        let in_src = path.starts_with("src");
        // .py files in exercise root are student source files
        let is_in_project_root = match path.parent() {
            Some(s) => s.as_os_str().is_empty(),
            None => true,
        };
        let is_py_file = path.extension() == Some(OsStr::new("py"));
        let is_ipynb = path.extension() == Some(OsStr::new("ipynb"));

        // all in all, excluding cache files and the exception subdirs,
        // we take non-cache files in src, py files in root, everything not in the root and not in src, and all ipynb files
        in_src
            || is_in_project_root && is_py_file
            || !is_in_exercise_subdir && !is_in_project_root
            || is_ipynb
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn in_src_is_source_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("src/some_file.py")));
        assert!(policy.is_student_file(Path::new("src/some_dir/some_file.py")));
    }

    #[test]
    fn in_root_is_source_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("some_file.py")));
    }

    #[test]
    fn pycache_is_not_source_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("__pycache__")));
        assert!(!policy.is_student_file(Path::new("__pycache__/cachefile")));
        assert!(!policy.is_student_file(Path::new("src/__pycache__")));
    }

    #[test]
    fn pyc_is_not_source_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("some.pyc")));
        assert!(!policy.is_student_file(Path::new("src/other.pyc")));
    }

    #[test]
    fn subdirs_are_student_files() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("subdir/something")));
        assert!(policy.is_student_file(Path::new("another/mid/else")));
    }

    #[test]
    fn tmc_and_test_are_not_student_files() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("test/something.py")));
        assert!(!policy.is_student_file(Path::new("tmc/mid/else.py")));
    }

    #[test]
    fn non_py_file_in_root_is_not_student_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("test")));
        assert!(!policy.is_student_file(Path::new("root_file")));
    }

    #[test]
    fn venv_dir_is_not_student_file() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("venv/asd.py")));
        assert!(!policy.is_student_file(Path::new(".venv/asd.py")));
    }
}
