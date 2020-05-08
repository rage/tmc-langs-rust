use isolang::Language;
use lazy_static::lazy_static;
use log::error;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    Error, LanguagePlugin,
};

pub enum LocalPy {
    Unix,
    Windows,
    WindowsConda(String),
}

lazy_static! {
    pub static ref LOCAL_PY: LocalPy = {
        if cfg!(windows) {
            // Check for Conda
            let conda = env::var("CONDA_PYTHON_EXE");
            if let Ok(conda_path) = conda {
                if PathBuf::from(&conda_path).exists() {
                    return LocalPy::WindowsConda(conda_path);
                }
            }
            LocalPy::Windows
        } else {
            LocalPy::Unix
        }
    };
}

pub struct Python3Plugin {}

impl Python3Plugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl LanguagePlugin for Python3Plugin {
    fn get_plugin_name(&self) -> &'static str {
        "python3"
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        let run_result = run_tmc_command(path, &["available_points"]);

        if let Err(error) = run_result {
            error!("Failed to scan exercise. {}", error);
        }

        let test_descs = parse_exercise_description(path)?;
        Ok(ExerciseDesc::new(exercise_name, test_descs))
    }

    fn run_tests(&self, path: &Path) -> RunResult {
        let run_result = run_tmc_command(path, &[]);

        if let Err(error) = run_result {
            error!("Failed to parse exercise description. {}", error);
        }

        parse_test_result(path).expect("error parsing test results") // TODO: handle
    }

    fn check_code_style(&self, _path: &Path, _locale: Language) -> Option<ValidationResult> {
        None
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        let mut setup = path.to_owned();
        setup.push("setup.py");

        let mut requirements = path.to_owned();
        requirements.push("requirements.txt");

        let mut test = path.to_owned();
        test.push("test");
        test.push("__init__.py");

        let mut tmc = path.to_owned();
        tmc.push("tmc");
        tmc.push("__main__.py");

        setup.exists() || requirements.exists() || test.exists() || tmc.exists()
    }

    fn clean(&self, _path: &Path) {
        // no op
    }
}

fn run_tmc_command(path: &Path, extra_args: &[&str]) -> Result<std::process::Output, Error> {
    let common_args = ["-m", "tmc"];
    let result = match &*LOCAL_PY {
        LocalPy::Unix => Command::new("python3")
            .args(&common_args)
            .args(extra_args)
            .current_dir(path)
            .output()?,
        LocalPy::Windows => Command::new("py")
            .args(&["-3"])
            .args(&common_args)
            .args(extra_args)
            .current_dir(path)
            .output()?,
        LocalPy::WindowsConda(conda_path) => Command::new(conda_path)
            .args(&common_args)
            .args(extra_args)
            .current_dir(path)
            .output()?,
    };
    Ok(result)
}

fn parse_exercise_description(path: &Path) -> Result<Vec<TestDesc>, Error> {
    let mut test_descs = vec![];
    let mut path = path.to_owned();
    path.push(".available_points.json");
    let file = File::open(path)?;
    let json: HashMap<String, Vec<String>> = match serde_json::from_reader(BufReader::new(file)) {
        Ok(json) => json,
        Err(error) => return Err(Error::Other(Box::new(error))),
    };
    for (key, value) in json {
        test_descs.push(TestDesc::new(key, value));
    }
    Ok(test_descs)
}

fn parse_test_result(path: &Path) -> Result<RunResult, Error> {
    let mut path = path.to_owned();
    path.push(".tmc_test_results.json");
    let results_file = File::open(path)?;
    let test_results: Vec<TestResult> = match serde_json::from_reader(BufReader::new(results_file))
    {
        Ok(test_results) => test_results,
        Err(error) => return Err(Error::Other(Box::new(error))),
    };

    let mut status = RunStatus::Passed;
    for result in &test_results {
        if !result.passed {
            status = RunStatus::TestsFailed;
        }
    }
    Ok(RunResult::new(status, test_results, HashMap::new()))
}
