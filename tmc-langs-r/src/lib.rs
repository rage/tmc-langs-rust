mod error;

use error::RError;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, TestDesc},
    Error, LanguagePlugin, StudentFilePolicy,
};

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct RPlugin {}

impl LanguagePlugin for RPlugin {
    fn get_plugin_name(&self) -> &str {
        "r"
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        // run available points command
        let args = if cfg!(windows) {
            &["-e", "\"library('tmcRtestrunner');run_available_points()\""]
        } else {
            &["-e", "library(tmcRtestrunner);run_available_points()"]
        };
        let out = Command::new("Rscript")
            .current_dir(path)
            .args(args)
            .output()
            .map_err(|e| RError::Command("Rscript", e))?;
        if !out.status.success() {
            return Err(RError::CommandStatus("Rscript", out.status).into());
        }

        // parse exercise desc
        let points_path = path.join(".available_points.json");
        let json_file = File::open(&points_path).map_err(|e| RError::Io(points_path.clone(), e))?;
        let test_descs: Vec<TestDesc> =
            serde_json::from_reader(json_file).map_err(|e| RError::Json(points_path, e))?;

        Ok(ExerciseDesc {
            name: exercise_name,
            tests: test_descs,
        })
    }

    fn run_tests(&self, path: &Path) -> Result<RunResult, Error> {
        // delete results json
        let results_path = path.join(".results.json");
        if results_path.exists() {
            fs::remove_file(&results_path).map_err(|e| RError::Io(results_path.clone(), e))?;
        }

        // run test command
        let args = if cfg!(windows) {
            &["-e", "\"library('tmcRtestrunner');run_tests()\""]
        } else {
            &["-e", "library(tmcRtestrunner);run_tests()"]
        };
        let out = Command::new("Rscript")
            .current_dir(path)
            .args(args)
            .output()
            .map_err(|e| RError::Command("Rscript", e))?;
        if !out.status.success() {
            return Err(RError::CommandStatus("Rscript", out.status).into());
        }

        // parse test result
        let json_file =
            File::open(&results_path).map_err(|e| RError::Io(results_path.clone(), e))?;
        let run_result =
            serde_json::from_reader(json_file).map_err(|e| RError::Json(results_path, e))?;
        Ok(run_result)
    }

    fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
        Box::new(RStudentFilePolicy::new(project_path.to_path_buf()))
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        path.join("R").exists() || path.join("tests/testthat").exists()
    }

    /// No operation for now. To be possibly implemented later: remove .Rdata, .Rhistory etc
    fn clean(&self, _path: &Path) -> Result<(), Error> {
        Ok(())
    }
}

pub struct RStudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl RStudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for RStudentFilePolicy {
    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        path.starts_with("R")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }
}
