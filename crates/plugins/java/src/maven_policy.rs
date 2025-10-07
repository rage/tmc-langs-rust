//! Maven student file policy

use std::{ffi::OsStr, path::Path};
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct MavenStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for MavenStudentFilePolicy {
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
        // technically pom.xml would need to be included to differentiate between maven and ant projects
        path.starts_with("src/main") && path.extension() == Some(OsStr::new("java"))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn is_student_file() {
        let policy = MavenStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(policy.is_student_file(Path::new("src/main/file.java")));
        assert!(policy.is_student_file(Path::new("src/main/dir/file.java")));
    }

    #[test]
    fn is_not_student_source_file() {
        let policy = MavenStudentFilePolicy::new(Path::new(".")).unwrap();
        assert!(!policy.is_student_file(Path::new("file")));
        assert!(!policy.is_student_file(Path::new("dir/src/main/file")));
        assert!(!policy.is_student_file(Path::new("srca/main/file")));
        assert!(!policy.is_student_file(Path::new("src/mainc/file")));
    }
}
