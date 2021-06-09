//! Contains the JupyterNotebookPlugin struct

use crate::error::JupyterNotebookError;
use crate::policy::JupyterNotebookStudentPolicy;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc_langs_framework::{
    nom::{self, error::VerboseError, IResult},
    CommandError, ExerciseDesc, LanguagePlugin, Output, RunResult, RunStatus, TestDesc, TestResult,
    TmcCommand, TmcError,
};
use tmc_langs_util::file_util;
use walkdir::WalkDir;

pub struct JupyterNotebookPlugin {}

impl JupyterNotebookPlugin {
    pub const fn new() -> Self {
        Self {}
    }

    fn run_tmc_command(
        path: &Path,
        extra_args: &[&str],
        timeout: Option<Duration>,
        _stdin: Option<String>,
    ) -> Result<Output, JupyterNotebookError> {
        // TODO: Check for errors

        let path = dunce::canonicalize(path)
            .map_err(|e| JupyterNotebookError::Canonicalize(path.to_path_buf(), e))?;
        log::debug!("running tmc command at {}", path.display());

        let command = TmcCommand::piped("nbgrader");
        let common_args = ["autograde", "exercise", "--force"];
        let command = command.with(|e| e.args(&common_args).args(extra_args).cwd(&path));

        let output = if let Some(timeout) = timeout {
            command.output_with_timeout(timeout)?
        } else {
            command.output()?
        };

        // Export results from db.
        TmcCommand::piped("nbgrader")
            .with(|e| e.args(&["export"]).cwd(&path))
            .output()?;

        log::trace!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        Ok(output)
    }

    fn parse_and_verify_test_result(
        test_results_csv: &Path,
        logs: HashMap<String, String>,
    ) -> Result<RunResult, JupyterNotebookError> {
        let results = file_util::read_file_to_string(&test_results_csv)?;

        let mut test_results: Vec<TestResult> = Vec::new();

        let mut status = RunStatus::Passed;
        for result in results.lines().filter_map(|r| {
            log::debug!("asd: {:?}", r);
            if r.starts_with("exercise") {
                Some(r.split('.').collect::<Vec<&str>>())
            } else {
                None
            }
        }) {
            log::debug!("asd: {:?}", result);
            let passed = result[9] == result[10];
            test_results.push(TestResult {
                name: "exercise".to_string(),
                successful: passed,
                points: vec![],
                message: "".to_string(),
                exception: vec![],
            });
            if result[9] != result[10] {
                status = RunStatus::TestsFailed;
            }
        }

        Ok(RunResult::new(status, test_results, logs))
    }
}

impl LanguagePlugin for JupyterNotebookPlugin {
    const PLUGIN_NAME: &'static str = "jupyter";
    const LINE_COMMENT: &'static str = "#";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("\"\"\"", "\"\"\""));
    type StudentFilePolicy = JupyterNotebookStudentPolicy;

    fn scan_exercise(
        &self,
        _exercise_directory: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, TmcError> {
        let test_name = format!("{}Test", exercise_name);
        Ok(ExerciseDesc {
            name: exercise_name,
            tests: vec![TestDesc {
                name: test_name,
                points: vec![],
            }],
        })
    }

    fn run_tests_with_timeout(
        &self,
        exercise_directory: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        let test_results_csv = exercise_directory.join("grades.csv");

        let output = Self::run_tmc_command(exercise_directory, &[], timeout, None);

        match output {
            Ok(_output) => {
                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), "".to_string());
                logs.insert("stderr".to_string(), "".to_string());

                let parse_res = Self::parse_and_verify_test_result(&test_results_csv, logs)?;

                Ok(parse_res)
            }
            Err(JupyterNotebookError::Tmc(TmcError::Command(CommandError::TimeOut {
                stdout,
                stderr,
                ..
            }))) => {
                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), stdout);
                logs.insert("stderr".to_string(), stderr);
                Ok(RunResult {
                    status: RunStatus::TestsFailed,
                    test_results: vec![TestResult {
                        name: "Timeout test".to_string(),
                        successful: false,
                        points: vec![],
                        message:
                            "Tests timed out.\nMake sure you don't have an infinite loop in your code."
                                .to_string(),
                        exception: vec![],
                    }],
                    logs,
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    fn is_exercise_type_correct(path: &Path) -> bool {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|entry| {
                entry.path().is_file() && entry.path().extension() == Some(OsStr::new("ipynb"))
            })
    }

    fn clean(&self, _exercise_path: &Path) -> Result<(), TmcError> {
        unimplemented!()
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        unimplemented!()
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        unimplemented!()
    }

    fn points_parser(_i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        // TODO: Implement.
        Err(nom::Err::Error(VerboseError { errors: vec![] }))
    }
}
