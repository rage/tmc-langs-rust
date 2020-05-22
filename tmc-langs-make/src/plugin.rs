use crate::check_log::CheckLog;
use crate::error::MakeError;
use crate::policy::MakeStudentFilePolicy;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc},
    plugin::LanguagePlugin,
    policy::StudentFilePolicy,
    Error,
};

pub struct MakePlugin {}

impl MakePlugin {
    pub fn new() -> Self {
        Self {}
    }

    fn parse_exercise_desc(
        &self,
        available_points: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, Error> {
        lazy_static! {
            static ref RE: Regex =
                Regex::new(r#"\[(?P<type>.*)\] \[(?P<name>.*)\] (?P<points>.*)"#).unwrap();
        }

        let mut tests = vec![];

        let file = File::open(available_points)
            .map_err(|e| MakeError::FileOpen(available_points.to_path_buf(), e))?;

        let reader = BufReader::new(file);
        for line in reader.lines().filter_map(|l| l.ok()) {
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

    fn run_tests_with_valgrind(&self, path: &Path, valgrind: bool) -> Result<(), Error> {
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
                return MakeError::ValgrindTests.into();
            } else {
                return MakeError::NoValgrindTests.into();
            }
        }

        Ok(())
    }

    fn builds(&self, dir: &Path) -> Result<bool, Error> {
        let output = Command::new("make")
            .current_dir(dir)
            .arg("test")
            .output()
            .map_err(|e| MakeError::MakeCommand(e))?;

        log::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));

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
            return MakeError::CantParseExerciseDesc.into();
        }

        self.parse_exercise_desc(&available_points_path, exercise_name)
    }

    fn run_tests(&self, path: &Path) -> Result<RunResult, Error> {
        if !self.builds(path)? {
            return Ok(RunResult {
                status: RunStatus::CompileFailed,
                test_results: vec![],
                logs: HashMap::new(),
            });
        }

        if let Err(e) = self.run_tests_with_valgrind(path, true) {
            log::error!("Running with valgrind failed! {}", e);
            todo!("run without valgrind")
        }

        let base_test_path = path.join("test");
        let test_results_path = base_test_path.join("tmc_test_results.xml");

        // parse .tmcproject.yml for fail_on_valgrind_error, default=true, this.valgrindStrategy
        // if valgrind was run and fail on valgrind error valgrindparser(valgrindoutput).addoutputs(tests) ??

        if !test_results_path.exists() {
            Ok(RunResult {
                status: RunStatus::CompileFailed,
                test_results: vec![],
                logs: HashMap::new(),
            })
        } else {
            let file = File::open(&test_results_path)?;
            let check_log: CheckLog = serde_xml_rs::from_reader(file).unwrap();
            log::debug!("{:?}", check_log);
            Ok(check_log.into())
        }
    }

    fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
        Box::new(MakeStudentFilePolicy::new(project_path.to_path_buf()))
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        path.join("Makefile").is_file()
    }

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
        panic!("{:?}", run_result);
    }
}
