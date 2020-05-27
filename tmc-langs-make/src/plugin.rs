//! Contains the main plugin struct.

use crate::check_log::CheckLog;
use crate::error::MakeError;
use crate::policy::MakeStudentFilePolicy;
use crate::valgrind_log::ValgrindLog;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TmcProjectYml},
    plugin::LanguagePlugin,
    policy::StudentFilePolicy,
    Error,
};

pub struct MakePlugin {}

impl MakePlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Parses tmc_available_points.txt which is output by the TMC tests and
    /// contains lines like "[test] [test_one] 1.1 1.2 1.3" = "[type] [name] points".
    fn parse_exercise_desc(
        &self,
        available_points: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, MakeError> {
        lazy_static! {
            // "[test] [test_one] 1.1 1.2 1.3" = "[type] [name] points"
            static ref RE: Regex =
                Regex::new(r#"\[(?P<type>.*)\] \[(?P<name>.*)\] (?P<points>.*)"#).unwrap();
        }

        let mut tests = vec![];

        let file = File::open(available_points)
            .map_err(|e| MakeError::FileOpen(available_points.to_path_buf(), e))?;

        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|e| MakeError::FileRead(available_points.to_path_buf(), e))?;

            if let Some(captures) = RE.captures(&line) {
                if &captures["type"] == "test" {
                    let name = captures["name"].to_string();
                    let points = captures["points"]
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect();
                    tests.push(TestDesc { name, points });
                }
            }
        }

        Ok(ExerciseDesc {
            name: exercise_name,
            tests,
        })
    }

    /// Runs tests with or without valgrind according to the argument.
    /// Returns an error if the command finishes unsuccessfully.
    fn run_tests_with_valgrind(&self, path: &Path, valgrind: bool) -> Result<(), MakeError> {
        let arg = if valgrind {
            "run-test-with-valgrind"
        } else {
            "run-test"
        };
        log::info!("Running make {}", arg);

        let output = Command::new("make")
            .current_dir(path)
            .arg(arg)
            .output()
            .map_err(|e| MakeError::MakeCommand(e))?;

        log::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            if valgrind {
                return Err(MakeError::ValgrindTests);
            } else {
                return Err(MakeError::NoValgrindTests);
            }
        }

        Ok(())
    }

    /// Tries to build the project at the given directory, returns whether
    /// the process finished successfully or not.
    fn builds(&self, dir: &Path) -> Result<bool, MakeError> {
        log::debug!("building {}", dir.display());
        let output = Command::new("make")
            .current_dir(dir)
            .arg("test")
            .output()
            .map_err(|e| MakeError::MakeCommand(e))?;

        log::debug!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));

        Ok(output.status.success())
    }
}

impl LanguagePlugin for MakePlugin {
    fn get_plugin_name(&self) -> &str {
        "make"
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        if !self.is_exercise_type_correct(path) {
            return MakeError::NoExerciseFound.into();
        }

        self.run_tests_with_valgrind(path, false)?;

        let available_points_path = path.join("test/tmc_available_points.txt");

        if !available_points_path.exists() {
            return MakeError::CantFindAvailablePoints.into();
        }

        Ok(self.parse_exercise_desc(&available_points_path, exercise_name)?)
    }

    fn run_tests(&self, path: &Path) -> Result<RunResult, Error> {
        if !self.builds(path)? {
            return Ok(RunResult {
                status: RunStatus::CompileFailed,
                test_results: vec![],
                logs: HashMap::new(),
            });
        }

        // try to run valgrind
        let mut ran_valgrind = true;
        let valgrind_run = self.run_tests_with_valgrind(path, true);
        if let Err(MakeError::MakeCommand(io_error)) = valgrind_run {
            if io_error.kind() == io::ErrorKind::PermissionDenied {
                // failed due to lacking permissions, try to clean and rerun
                let _output = self.clean(path)?;
                if let Err(err) = self.run_tests_with_valgrind(path, false) {
                    log::error!(
                        "Running with valgrind failed after trying to clean! {}",
                        err
                    );
                    ran_valgrind = false;
                    log::info!("Running without valgrind");
                    self.run_tests_with_valgrind(path, false)?;
                }
            } else {
                // failed due to something else, valgrind might not be installed
                ran_valgrind = false;
                log::info!("Running without valgrind");
                self.run_tests_with_valgrind(path, false)?;
            }
        }

        let base_test_path = path.join("test");

        // fails on valgrind by default
        let fail_on_valgrind_error =
            match TmcProjectYml::from(&base_test_path.join(".tmcproject.yml")) {
                Ok(parsed) => parsed.fail_on_valgrind_error.unwrap_or(true),
                Err(_) => true,
            };

        // valgrind logs are only interesting if fail on valgrind error is on
        let valgrind_log = if ran_valgrind && fail_on_valgrind_error {
            let valgrind_path = base_test_path.join("valgrind.log");
            Some(ValgrindLog::from(&valgrind_path)?)
        } else {
            None
        };

        // parse available points into a mapping from test name to test point list
        let available_points_path = base_test_path.join("tmc_available_points.txt");
        let tests = self
            .parse_exercise_desc(&available_points_path, "unused".to_string())?
            .tests;
        let mut ids_to_points = HashMap::new();
        for test in tests {
            ids_to_points.insert(test.name, test.points);
        }

        // parse test results into RunResult
        let test_results_path = base_test_path.join("tmc_test_results.xml");
        let file = File::open(&test_results_path)
            .map_err(|e| MakeError::FileOpen(test_results_path.clone(), e))?;
        let check_log: CheckLog = serde_xml_rs::from_reader(file)
            .map_err(|e| MakeError::XmlParseError(test_results_path, e))?;
        let mut run_result = check_log.into_run_result(ids_to_points);

        if let Some(valgrind_log) = valgrind_log {
            if valgrind_log.errors {
                // valgrind failed
                run_result.status = RunStatus::TestsFailed;
                // TODO: tests and valgrind results are not guaranteed to be in the same order
                for (test_result, valgrind_result) in run_result
                    .test_results
                    .iter_mut()
                    .zip(valgrind_log.results.into_iter())
                {
                    if valgrind_result.errors {
                        if test_result.passed {
                            test_result.message += " - Failed due to errors in valgrind log; see log below. Try submitting to server, some leaks might be platform dependent";
                        }
                        test_result.exceptions.extend(valgrind_result.log);
                    }
                }
            }
        }

        Ok(run_result)
    }

    fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
        Box::new(MakeStudentFilePolicy::new(project_path.to_path_buf()))
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        path.join("Makefile").is_file()
    }

    // does not check for success
    fn clean(&self, path: &Path) -> Result<(), Error> {
        let output = Command::new("make")
            .current_dir(path)
            .arg("clean")
            .output()
            .map_err(|e| MakeError::MakeCommand(e))?;

        if output.status.success() {
            log::info!("Cleaned make project");
        } else {
            log::warn!("Cleaning make project was not successful");
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    // copies the target exercise and tmc to a temp directory
    fn copy_test(dir: &str) -> TempDir {
        let path = Path::new(dir);
        let temp = tempdir().unwrap();
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
    fn scans_exercise() {
        init();

        let temp = copy_test("tests/data/passing");
        let plugin = MakePlugin::new();
        let exercise_desc = plugin
            .scan_exercise(temp.path(), "test".to_string())
            .unwrap();

        assert_eq!(exercise_desc.name, "test");
        assert_eq!(exercise_desc.tests.len(), 1);
        let test = &exercise_desc.tests[0];
        assert_eq!(test.name, "test_one");
        assert_eq!(test.points.len(), 1);
        assert_eq!(test.points[0], "1.1");
    }

    #[test]
    fn runs_tests() {
        init();

        let temp = copy_test("tests/data/passing");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        assert!(run_result.logs.is_empty());
        let test_results = run_result.test_results;
        assert_eq!(test_results.len(), 1);
        let test_result = &test_results[0];
        assert_eq!(test_result.name, "test_one");
        assert!(test_result.passed);
        assert_eq!(test_result.message, "Passed");
        assert!(test_result.exceptions.is_empty());
        let points = &test_result.points;
        assert_eq!(points.len(), 1);
        let point = &points[0];
        assert_eq!(point, "1.1");
    }

    #[test]
    fn runs_tests_failing() {
        init();

        let temp = copy_test("tests/data/failing");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        let test_results = &run_result.test_results;
        assert_eq!(test_results.len(), 1);
        let test_result = &test_results[0];
        assert_eq!(test_result.name, "test_one");
        assert!(!test_result.passed);
        assert!(test_result.message.contains("Should have returned: 1"));
        let points = &test_result.points;
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], "1.1");
        let logs = &run_result.logs;
        assert!(logs.is_empty());
    }

    //#[test]
    fn runs_tests_failing_valgrind() {
        init();

        let temp = copy_test("tests/data/valgrind-failing");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        let test_results = &run_result.test_results;
        assert_eq!(test_results.len(), 2);

        let test_one = &test_results[0];
        assert_eq!(test_one.name, "test_one");
        assert!(test_one.passed);
        assert_eq!(test_one.points.len(), 1);
        assert_eq!(test_one.points[0], "1.1");

        let test_two = &test_results[1];
        assert_eq!(test_two.name, "test_two");
        assert!(test_two.passed);
        assert_eq!(test_two.points.len(), 1);
        assert_eq!(test_two.points[0], "1.2");

        todo!("valgrind results in random order?")
    }
}
