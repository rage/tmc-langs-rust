//! Contains StudentFilePolicy.

use super::TmcProjectYml;
use crate::TmcError;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Specifies which files are student files.
///
/// Student files are any files that are expected to be modified and/or created by the student.
/// That is, any files that should not be overwritten when when updating an already downloaded
/// exercise and any files that should be submitted to the server.
pub trait StudentFilePolicy {
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
        tmc_project_yml: &TmcProjectYml,
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
        log::debug!("{} {}", path_canon.display(), root_canon.display());

        // the project root path should be considered a student file
        if path_canon == root_canon {
            return Ok(true);
        }

        // strip root directory from file path
        let relative = path_canon
            .strip_prefix(&root_canon)
            .map_err(|_| TmcError::FileNotInProject(path_canon.clone(), root_canon.clone()))?;
        Ok(self.is_extra_student_file(&relative, tmc_project_yml)?
            || self.is_student_source_file(relative))
    }

    fn get_config_file_parent_path(&self) -> &Path;

    fn get_tmc_project_yml(&self) -> Result<TmcProjectYml, TmcError> {
        TmcProjectYml::from(self.get_config_file_parent_path())
    }

    /// Determines whether a file is an extra student file.
    ///
    /// The file_path should be relative, starting from the project root.
    fn is_extra_student_file(
        &self,
        file_path: &Path,
        tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        for extra_student_file in &tmc_project_yml.extra_student_files {
            if file_path.starts_with(extra_student_file) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Checks whether the file is a student source file. The file_path can be assumed to be a relative path starting from the project root directory.
    fn is_student_source_file(&self, file_path: &Path) -> bool;

    /// Used to check for files which should always be overwritten.
    ///
    /// The file_path should be relative, starting from the project root.
    fn is_updating_forced(
        &self,
        path: &Path,
        tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        for force_update_path in &tmc_project_yml.force_update {
            if path.starts_with(force_update_path) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub struct NothingIsStudentFilePolicy {}

impl StudentFilePolicy for NothingIsStudentFilePolicy {
    fn is_student_file(
        &self,
        _path: &Path,
        _project_root_path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        Ok(false)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        Path::new("")
    }

    fn is_extra_student_file(
        &self,
        _path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        unimplemented!()
    }

    fn is_student_source_file(&self, _path: &Path) -> bool {
        unimplemented!()
    }

    fn is_updating_forced(
        &self,
        _path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        Ok(false)
    }
}

pub struct EverythingIsStudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl EverythingIsStudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for EverythingIsStudentFilePolicy {
    fn is_student_file(
        &self,
        _path: &Path,
        _project_root_path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        Ok(true)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }

    fn is_extra_student_file(
        &self,
        _path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        unimplemented!()
    }

    fn is_student_source_file(&self, _path: &Path) -> bool {
        unimplemented!()
    }

    fn is_updating_forced(
        &self,
        _path: &Path,
        _tmc_project_yml: &TmcProjectYml,
    ) -> Result<bool, TmcError> {
        Ok(false)
    }
}
