pub mod sandbox;
pub mod zip;

use super::{Result, TmcProjectYml};
use std::ffi::OsStr;
use std::path::Path;

pub trait StudentFilePolicy {
    fn is_student_file(&self, path: &Path, project_root_path: &Path) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }

        if path.file_name() == Some(OsStr::new(".tmcproject.yml")) {
            return Ok(false);
        }

        return Ok(self.is_extra_student_file(path)?
            || project_root_path == path
            || self.is_student_source_file(path));
    }

    fn get_config_file_parent_path(&self) -> &Path;

    fn is_extra_student_file(&self, path: &Path) -> Result<bool> {
        let absolute = path.canonicalize()?;
        let tmc_project_yml = TmcProjectYml::from(self.get_config_file_parent_path())?;
        for path in tmc_project_yml.extra_exercise_files {
            let path = path.canonicalize()?;
            if path.is_dir() && (path == absolute || absolute.starts_with(path)) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn is_student_source_file(&self, path: &Path) -> bool;

    fn is_updating_forced(&self, path: &Path) -> Result<bool> {
        let absolute = path.canonicalize()?;
        let tmc_project_yml = TmcProjectYml::from(self.get_config_file_parent_path())?;
        for force_update_path in tmc_project_yml.force_update {
            let force_absolute = force_update_path.canonicalize()?;
            if (absolute == force_absolute || absolute.starts_with(&force_absolute))
                && force_absolute.is_dir()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub struct NothingIsStudentFilePolicy {}

impl StudentFilePolicy for NothingIsStudentFilePolicy {
    fn is_student_file(&self, path: &Path, project_root_path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        unimplemented!()
    }

    fn is_extra_student_file(&self, path: &Path) -> Result<bool> {
        unimplemented!()
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        unimplemented!()
    }
}

pub struct EverythingIsStudentFilePolicy {}

impl StudentFilePolicy for EverythingIsStudentFilePolicy {
    fn is_student_file(&self, path: &Path, project_root_path: &Path) -> Result<bool> {
        Ok(true)
    }

    fn get_config_file_parent_path(&self) -> &Path {
        unimplemented!()
    }

    fn is_extra_student_file(&self, path: &Path) -> Result<bool> {
        unimplemented!()
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        unimplemented!()
    }
}
