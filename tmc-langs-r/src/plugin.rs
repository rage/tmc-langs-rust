// Contains the RPlugin

use crate::error::RError;
use crate::RStudentFilePolicy;

use super::RRunResult;

use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, TestDesc},
    LanguagePlugin, TmcError,
};

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub struct RPlugin {}

impl RPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl LanguagePlugin for RPlugin {
    const PLUGIN_NAME: &'static str = "r";
    type StudentFilePolicy = RStudentFilePolicy;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
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
            return Err(RError::CommandStatus(
                "Rscript",
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
            .into());
        }
        log::trace!("stdout: {}", String::from_utf8_lossy(&out.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&out.stderr));

        // parse exercise desc
        let points_path = path.join(".available_points.json");
        let json_file =
            File::open(&points_path).map_err(|e| RError::FileOpen(points_path.clone(), e))?;
        let test_descs: HashMap<String, Vec<String>> = serde_json::from_reader(json_file)
            .map_err(|e| RError::JsonDeserialize(points_path, e))?;
        let test_descs = test_descs
            .into_iter()
            .map(|(k, v)| TestDesc { name: k, points: v })
            .collect();

        Ok(ExerciseDesc {
            name: exercise_name,
            tests: test_descs,
        })
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        // delete results json
        let results_path = path.join(".results.json");
        if results_path.exists() {
            fs::remove_file(&results_path)
                .map_err(|e| RError::FileRemove(results_path.clone(), e))?;
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

        log::trace!("stdout: {}", String::from_utf8_lossy(&out.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&out.stderr));

        if !out.status.success() {
            return Err(RError::CommandStatus(
                "Rscript",
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
            .into());
        }

        // parse test result
        let json_file =
            File::open(&results_path).map_err(|e| RError::FileOpen(results_path.clone(), e))?;
        let run_result: RRunResult = serde_json::from_reader(json_file).map_err(|e| {
            if let Ok(s) = fs::read_to_string(&results_path) {
                log::error!("json {}", s);
            }
            RError::JsonDeserialize(results_path, e)
        })?;

        Ok(run_result.into())
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        RStudentFilePolicy::new(project_path.to_path_buf())
    }

    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("R").exists() || path.join("tests/testthat").exists()
    }

    /// No operation for now. To be possibly implemented later: remove .Rdata, .Rhistory etc
    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")] // tmc-r-testrunner not installed on other CI platforms
mod test {
    use super::*;
    use std::path::PathBuf;
    use tmc_langs_framework::domain::RunStatus;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    // copies the target exercise and tmc to a temp directory
    fn copy_test(dir: &str) -> tempfile::TempDir {
        let path = Path::new(dir);
        let temp = tempfile::tempdir().unwrap();
        for entry in walkdir::WalkDir::new(path) {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let entry_path: PathBuf = entry
                    .path()
                    .components()
                    .skip(path.components().count())
                    .collect();
                let temp_path = temp.path().join(entry_path);
                temp_path
                    .parent()
                    .map(|p| std::fs::create_dir_all(&p).unwrap());
                log::trace!("copying {:?} -> {:?}", entry.path(), temp_path);
                std::fs::copy(entry.path(), temp_path).unwrap();
            }
        }
        temp
    }

    #[test]
    fn scan_exercise() {
        init();
        let plugin = RPlugin {};
        let temp = copy_test("tests/data/simple_all_tests_pass");

        assert!(!temp.path().join(".available_points.json").exists());
        let desc = plugin.scan_exercise(temp.path(), "ex".to_string()).unwrap();
        assert!(temp.path().join(".available_points.json").exists());
        assert_eq!(desc.name, "ex");
        assert_eq!(desc.tests.len(), 4);
        for test in desc.tests {
            if test.name == "ret_true works." {
                assert_eq!(test.points.len(), 2);
                assert_eq!(test.points[0], "r1");
                return;
            }
        }
        panic!("not found");
    }

    #[test]
    fn run_tests_success() {
        init();
        let plugin = RPlugin {};
        let temp = copy_test("tests/data/simple_all_tests_pass");

        let run = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run.status, RunStatus::Passed);
        assert!(run.logs.is_empty());
        assert_eq!(run.test_results.len(), 4);
        for res in run.test_results {
            if res.name == "ret_true works." {
                assert!(res.successful);
                assert_eq!(res.points, &["r1", "r1.1"]);
                assert!(res.message.is_empty());
                assert!(res.exception.is_empty());
                return;
            }
        }
        panic!("not found");
    }

    #[test]
    fn run_tests_failed() {
        init();
        let plugin = RPlugin {};
        let temp = copy_test("tests/data/simple_all_tests_fail");

        let run = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run.status, RunStatus::TestsFailed);
        assert!(run.logs.is_empty());
        assert_eq!(run.test_results.len(), 4);
        for res in run.test_results {
            if res.name == "ret_true works." {
                assert!(!res.successful);
                assert_eq!(res.points, &["r1", "r1.1"]);
                assert!(!res.message.is_empty());
                assert!(res.exception.is_empty());
                return;
            }
        }
        panic!("not found");
    }

    #[test]
    fn run_tests_run_failed() {
        init();
        let plugin = RPlugin {};
        let temp = copy_test("tests/data/simple_run_fail");

        let mut run = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run.status, RunStatus::CompileFailed);
        assert!(run.test_results.is_empty());
        assert!(!run.logs.is_empty());
        let logs = run.logs.remove("compiler_output").unwrap();
        let logs = String::from_utf8(logs).unwrap();
        assert!(logs.contains("unexpected 'in'"))
    }

    #[test]
    fn run_tests_sourcing() {
        init();
        let plugin = RPlugin {};
        let temp = copy_test("tests/data/simple_sourcing_fail");

        let mut run = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run.status, RunStatus::CompileFailed);
        assert!(run.test_results.is_empty());
        assert!(!run.logs.is_empty());
        let logs = run.logs.remove("compiler_output").unwrap();
        let logs = String::from_utf8(logs).unwrap();
        assert!(logs.contains("unexpected 'in'"));
    }
}
