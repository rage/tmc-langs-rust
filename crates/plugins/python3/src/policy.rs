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
        // never include pyc files
        let is_pyc = path.extension() == Some(OsStr::new("pyc"));
        if is_pyc {
            return false;
        }

        let in_src = path.starts_with("src");
        let in_exercise_root = match path.parent() {
            Some(s) => s.as_os_str().is_empty(),
            None => true,
        };
        let is_py = path.extension() == Some(OsStr::new("py"));
        let is_ipynb = path.extension() == Some(OsStr::new("ipynb"));
        (in_src || in_exercise_root) && (is_py || is_ipynb)
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
    fn subdirs_are_not_student_files() {
        let policy = Python3StudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("subdir/something")));
        assert!(!policy.is_student_file(Path::new("another/mid/else")));
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
