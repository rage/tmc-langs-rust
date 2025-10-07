//! Ant student file policy

use std::{ffi::OsStr, path::Path};
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct AntStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for AntStudentFilePolicy {
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
        path.starts_with("src") && path.extension() == Some(OsStr::new("java"))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn is_student_file() {
        let policy = AntStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("src/file.java")));
        assert!(policy.is_student_file(Path::new("src/dir/file.java")));
    }

    #[test]
    fn is_not_student_source_file() {
        let policy = AntStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("src/file")));
        assert!(!policy.is_student_file(Path::new("file")));
        assert!(!policy.is_student_file(Path::new("dir/src/file")));
        assert!(!policy.is_student_file(Path::new("srca/file")));
    }
}
