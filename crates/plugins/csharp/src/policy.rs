//! Student file policy for the C# plugin.

use std::{ffi::OsStr, path::Path};
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct CSharpStudentFilePolicy {
    project_config: TmcProjectYml,
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

    // .cs files in src
    fn is_non_extra_student_file(&self, path: &Path) -> bool {
        path.starts_with("src") && path.extension() == Some(OsStr::new("cs"))
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
