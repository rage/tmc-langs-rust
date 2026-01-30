//! Contains the language policy for the plugin.

use std::{ffi::OsStr, path::Path};
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct MakeStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for MakeStudentFilePolicy {
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
        let ext = path.extension();
        path.starts_with("src") && (ext == Some(OsStr::new("c")) || ext == Some(OsStr::new("h")))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn is_student_file() {
        let policy = MakeStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("src/file.c")));
        assert!(policy.is_student_file(Path::new("src/file.h")));
        assert!(policy.is_student_file(Path::new("src/dir/file.c")));
        assert!(policy.is_student_file(Path::new("src/dir/file.h")));
    }

    #[test]
    fn is_not_student_source_file() {
        let policy = MakeStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("a.c")));
        assert!(!policy.is_student_file(Path::new("a.h")));
        assert!(!policy.is_student_file(Path::new("srcc")));
        assert!(!policy.is_student_file(Path::new("dir/src/file")));
    }
}
