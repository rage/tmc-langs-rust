//! Implementation of LanguagePlugin for Python 3.

use isolang::Language;
use lazy_static::lazy_static;
use log::{debug, error};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    plugin::LanguagePlugin,
    policy::StudentFilePolicy,
    Error,
};

pub enum LocalPy {
    Unix,
    Windows,
    WindowsConda(String),
}

lazy_static! {
    // the python command is platform-dependent
    pub static ref LOCAL_PY: LocalPy = {
        if cfg!(windows) {
            // Check for Conda
            let conda = env::var("CONDA_PYTHON_EXE");
            if let Ok(conda_path) = conda {
                if PathBuf::from(&conda_path).exists() {
                    debug!("detected conda on windows");
                    return LocalPy::WindowsConda(conda_path);
                }
            }
            debug!("detected windows");
            LocalPy::Windows
        } else {
            debug!("detected unix");
            LocalPy::Unix
        }
    };
}

pub struct Python3Plugin {}

impl Python3Plugin {
    pub const fn new() -> Self {
        Self {}
    }
}

impl LanguagePlugin for Python3Plugin {
    fn get_plugin_name(&self) -> &'static str {
        "python3"
    }

    fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
        Box::new(Python3StudentFilePolicy::new(project_path.to_owned()))
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        let run_result = run_tmc_command(path, &["available_points"]);

        if let Err(error) = run_result {
            error!("Failed to scan exercise. {}", error);
        }

        let test_descs = parse_exercise_description(path)?;
        Ok(ExerciseDesc::new(exercise_name, test_descs))
    }

    fn run_tests(&self, path: &Path) -> Result<RunResult, Error> {
        let run_result = run_tmc_command(path, &[]);

        if let Err(error) = run_result {
            error!("Failed to parse exercise description. {}", error);
        }

        Ok(parse_test_result(path)?)
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

    fn clean(&self, _path: &Path) -> Result<(), Error> {
        // no op
        Ok(())
    }
}

fn run_tmc_command(path: &Path, extra_args: &[&str]) -> Result<std::process::Output, Error> {
    let path = path.canonicalize()?;
    debug!("running tmc command at {:?}", path);
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
    // TODO: deserialize directly into Vec<TestDesc>?
    let json: HashMap<String, Vec<String>> = match serde_json::from_reader(BufReader::new(file)) {
        Ok(json) => json,
        Err(error) => return Err(Error::Plugin(Box::new(error))),
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
        Err(error) => return Err(Error::Plugin(Box::new(error))),
    };

    let mut status = RunStatus::Passed;
    for result in &test_results {
        if !result.passed {
            status = RunStatus::TestsFailed;
        }
    }
    Ok(RunResult::new(status, test_results, HashMap::new()))
}

pub struct Python3StudentFilePolicy {
    config_file_parent_path: PathBuf,
}

impl Python3StudentFilePolicy {
    pub fn new(config_file_parent_path: PathBuf) -> Self {
        Self {
            config_file_parent_path,
        }
    }
}

impl StudentFilePolicy for Python3StudentFilePolicy {
    fn get_config_file_parent_path(&self) -> &Path {
        &self.config_file_parent_path
    }

    fn is_student_source_file(&self, path: &Path) -> bool {
        path.starts_with("src")
    }
}

#[cfg(test)]
mod test {
    use super::*;
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
        for entry in walkdir::WalkDir::new("testdata/tmc") {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let entry_path: PathBuf = entry.path().components().skip(2).collect();
                let temp_path = temp.path().join("tmc").join(entry_path);
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

        let temp = copy_test("testdata/project");
        let plugin = Python3Plugin::new();
        let ex_desc = plugin
            .scan_exercise(Path::new(temp.path()), "name".into())
            .unwrap();
        assert_eq!(ex_desc.name, "name");
        assert_eq!(
            &ex_desc.tests[0].name,
            "test.test_points.TestEverything.test_new"
        );
        assert!(ex_desc.tests[0].points.contains(&"1.1".into()));
        assert!(ex_desc.tests[0].points.contains(&"1.2".into()));
        assert!(ex_desc.tests[0].points.contains(&"2.2".into()));
        assert_eq!(ex_desc.tests[0].points.len(), 3);
    }

    #[test]
    fn runs_tests() {
        init();
        let plugin = Python3Plugin::new();

        let temp = copy_test("testdata/project");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_points.TestEverything.test_new"
        );
        assert!(run_result.test_results[0].passed);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert_eq!(run_result.test_results[0].points.len(), 3);
        assert!(run_result.test_results[0].message.is_empty());
        assert!(run_result.test_results[0].exceptions.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        assert!(run_result.logs.is_empty());

        let temp = copy_test("testdata/failing");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_failing.TestFailing.test_new"
        );
        assert!(!run_result.test_results[0].passed);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert!(run_result.test_results[0].message.starts_with("'a' != 'b'"));
        assert!(run_result.test_results[0].exceptions.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        assert!(run_result.logs.is_empty());

        let temp = copy_test("testdata/erroring");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_erroring.TestErroring.test_erroring"
        );
        assert!(!run_result.test_results[0].passed);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert_eq!(
            run_result.test_results[0].message,
            "name 'doSomethingIllegal' is not defined"
        );
        assert!(run_result.test_results[0].exceptions.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        assert!(run_result.logs.is_empty());
    }

    #[test]
    fn exercise_type_is_correct() {
        init();
        let plugin = Python3Plugin::new();

        let correct = plugin.is_exercise_type_correct(Path::new("testdata"));
        assert!(correct);

        let correct = plugin.is_exercise_type_correct(Path::new("./"));
        assert!(!correct);
    }
}
