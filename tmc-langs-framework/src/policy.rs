//! Contains StudentFilePolicy.

use crate::TmcError;
use crate::TmcProjectYml;
use std::ffi::OsStr;
use std::path::Path;

/// Specifies which files are student files. A single StudentFilePolicy is only valid for a single project as it uses a config file to determine its output.
///
/// Student files are any files that are expected to be modified and/or created by the student.
/// That is, any files that should not be overwritten when when updating an already downloaded
/// exercise and any files that should be submitted to the server.
pub trait StudentFilePolicy {
    /// This constructor should store the project config in the implementing struct.
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized;

    /// Parses a project config and calls the helper constructor. Implementing types should only be constructed using this function.
    fn new(project_dir: &Path) -> Result<Self, TmcError>
    where
        Self: Sized,
    {
        let project_config = TmcProjectYml::from(project_dir)?;
        Ok(Self::new_with_project_config(project_config))
    }

    /// The policy should contain a TmcProjectYml parsed from the project this policy was created for.
    fn get_project_config(&self) -> &TmcProjectYml;

    /// Checks whether the path is considered a student source file. The file_path can be assumed to be a relative path starting from the project root directory.
    fn is_student_source_file(file_path: &Path) -> bool;

    /// Determines whether a file is a student source file.
    ///
    /// A file should be considered a student source file if it resides in a location the student
    /// is expected to create his or her own source files in the general case. Any special cases
    /// are specified as ExtraStudentFiles in a separate configuration.
    ///
    /// For example in a Java project that uses Apache Ant, should return `true` for any files in the `src` directory.
    ///
    /// The file_path should be some file inside project_root_path.
    ///
    /// # Errors
    /// Returns an error if project_root_path doesn't exist, or if file_path is not in project_root_path.
    // TODO: look at removing project_root_path and just requiring file_path to be relative
    fn is_student_file(
        &self,
        file_path: &Path,
        project_root_path: &Path,
    ) -> Result<bool, TmcError> {
        // non-existent files and .tmcproject.yml should never be considered student files
        if !file_path.exists() || file_path.file_name() == Some(OsStr::new(".tmcproject.yml")) {
            return Ok(false);
        }

        // make paths absolute
        let path_canon = file_path
            .canonicalize()
            .map_err(|e| TmcError::Canonicalize(file_path.to_path_buf(), e))?;
        let root_canon = project_root_path
            .canonicalize()
            .map_err(|e| TmcError::Canonicalize(project_root_path.to_path_buf(), e))?;

        // the project root path should be considered a student file
        if path_canon == root_canon {
            return Ok(true);
        }

        // strip root directory from file path
        let relative = path_canon
            .strip_prefix(&root_canon)
            .map_err(|_| TmcError::FileNotInProject(path_canon.clone(), root_canon.clone()))?;

        log::debug!("relat {}", relative.display());

        // check extra student files
        let is_extra_student_file = self
            .get_project_config()
            .extra_student_files
            .iter()
            .any(|f| relative.starts_with(f));

        Ok(is_extra_student_file || Self::is_student_source_file(relative))
    }

    /// Used to check for files which should always be overwritten.
    ///
    /// The file_path should be relative, starting from the project root.
    fn is_updating_forced(&self, path: &Path) -> Result<bool, TmcError> {
        for force_update_path in &self.get_project_config().force_update {
            if path.starts_with(force_update_path) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Mock policy that ignores the config file and returns false for all files.
// TODO: remove
pub struct NothingIsStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for NothingIsStudentFilePolicy {
    fn new(_project_dir: &Path) -> Result<Self, TmcError>
    where
        Self: Sized,
    {
        let project_config = TmcProjectYml {
            ..Default::default()
        };
        Ok(Self { project_config })
    }

    fn new_with_project_config(_project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        let project_config = TmcProjectYml {
            ..Default::default()
        };
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    fn is_student_file(&self, _path: &Path, _project_root_path: &Path) -> Result<bool, TmcError> {
        Ok(false)
    }

    fn is_student_source_file(_path: &Path) -> bool {
        false
    }
}

/// Mock policy that ignores the config file and returns true for all files.
// TODO: remove
#[derive(Default)]
pub struct EverythingIsStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for EverythingIsStudentFilePolicy {
    fn new(_project_dir: &Path) -> Result<Self, TmcError>
    where
        Self: Sized,
    {
        let project_config = TmcProjectYml {
            ..Default::default()
        };
        Ok(Self { project_config })
    }

    fn new_with_project_config(_project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        let project_config = TmcProjectYml {
            ..Default::default()
        };
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    fn is_student_file(&self, _path: &Path, _project_root_path: &Path) -> Result<bool, TmcError> {
        Ok(true)
    }

    fn is_student_source_file(_path: &Path) -> bool {
        true
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::{Path, PathBuf};

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    struct MockPolicy {
        project_config: TmcProjectYml,
    }

    impl StudentFilePolicy for MockPolicy {
        fn new_with_project_config(project_config: TmcProjectYml) -> Self
        where
            Self: Sized,
        {
            Self { project_config }
        }

        fn get_project_config(&self) -> &TmcProjectYml {
            &self.project_config
        }

        fn is_student_source_file(file_path: &Path) -> bool {
            file_path
                .components()
                .any(|c| c.as_os_str() == "student_file")
        }
    }

    #[test]
    fn considers_student_source_files() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/student_file/some file", "");
        file_to(&temp, "other dir/student_file", "");
        file_to(&temp, "other dir/other file", "");
        file_to(&temp, "other file", "");

        let project_config = TmcProjectYml::default();
        let policy = MockPolicy { project_config };
        assert!(policy
            .is_student_file(&temp.path().join("dir/student_file/some file"), temp.path())
            .unwrap());
        assert!(policy
            .is_student_file(&temp.path().join("other dir/student_file"), temp.path())
            .unwrap());
        assert!(!policy
            .is_student_file(&temp.path().join("other dir/other file"), temp.path())
            .unwrap());
        assert!(!policy
            .is_student_file(&temp.path().join("other file"), temp.path())
            .unwrap());
    }

    #[test]
    fn considers_extra_student_files() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "sdir/some file", "");
        file_to(&temp, "other dir/sfile", "");
        file_to(&temp, "other dir/other file", "");
        file_to(&temp, "other file", "");

        let project_config = TmcProjectYml {
            extra_student_files: vec![PathBuf::from("sdir"), PathBuf::from("other dir/sfile")],
            ..Default::default()
        };
        let policy = MockPolicy { project_config };
        assert!(policy
            .is_student_file(&temp.path().join("sdir/some file"), temp.path())
            .unwrap());
        assert!(policy
            .is_student_file(&temp.path().join("other dir/sfile"), temp.path())
            .unwrap());
        assert!(!policy
            .is_student_file(&temp.path().join("other dir/other file"), temp.path())
            .unwrap());
        assert!(!policy
            .is_student_file(&temp.path().join("other file"), temp.path())
            .unwrap());
    }

    #[test]
    fn considers_force_uodate_paths() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "sdir/some file", "");
        file_to(&temp, "other dir/sfile", "");
        file_to(&temp, "other dir/other file", "");
        file_to(&temp, "other file", "");

        let project_config = TmcProjectYml {
            force_update: vec![PathBuf::from("sdir"), PathBuf::from("other dir/sfile")],
            ..Default::default()
        };
        let policy = MockPolicy { project_config };
        assert!(policy
            .is_updating_forced(Path::new("sdir/some file"))
            .unwrap());
        assert!(policy
            .is_updating_forced(Path::new("other dir/sfile"))
            .unwrap());
        assert!(!policy
            .is_updating_forced(Path::new("other dir/other file"))
            .unwrap());
        assert!(!policy.is_updating_forced(Path::new("other file")).unwrap());
    }
}
