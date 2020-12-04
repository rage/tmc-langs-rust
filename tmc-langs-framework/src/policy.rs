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
    /// Returns an error if either the file_path or project_root_path don't exist, or if file_path is not in project_root_path.
    fn is_student_file(
        &self,
        file_path: &Path,
        project_root_path: &Path,
    ) -> Result<bool, TmcError> {
        // non-existent files and .tmcproject.yml should never be considered student files
        if !file_path.exists() || file_path.file_name() == Some(OsStr::new(".tmcproject.yml")) {
            return Ok(false);
        }

        // check extra student files
        let is_extra_student_file = self
            .get_project_config()
            .extra_student_files
            .iter()
            .any(|f| file_path.starts_with(f));
        if is_extra_student_file {
            return Ok(true);
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

        Ok(Self::is_student_source_file(relative))
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

pub struct NothingIsStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for NothingIsStudentFilePolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
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

#[derive(Default)]
pub struct EverythingIsStudentFilePolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for EverythingIsStudentFilePolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
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
