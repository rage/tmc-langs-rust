use crate::error::PythonError;
use crate::policy::Python3StudentFilePolicy;
use crate::{LocalPy, PythonTestResult, LOCAL_PY};

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    plugin::LanguagePlugin,
    policy::StudentFilePolicy,
    CommandWithTimeout, Error,
};
use walkdir::WalkDir;

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
        let run_result = run_tmc_command(path, &["available_points"], None);

        if let Err(error) = run_result {
            log::error!("Failed to scan exercise. {}", error);
        }

        let test_descs = parse_exercise_description(path)?;
        Ok(ExerciseDesc::new(exercise_name, test_descs))
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, Error> {
        let run_result = run_tmc_command(path, &[], timeout);

        if let Err(error) = run_result {
            log::error!("Failed to parse exercise description. {}", error);
        }

        Ok(parse_test_result(path)?)
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

    fn clean(&self, path: &Path) -> Result<(), Error> {
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_name() == ".available_points.json"
                || entry.file_name() == ".tmc_test_results.json"
            {
                fs::remove_file(entry.path())
                    .map_err(|e| PythonError::FileRemove(entry.path().to_path_buf(), e))?;
            } else if entry.file_name() == "__pycache__" {
                fs::remove_dir_all(entry.path())
                    .map_err(|e| PythonError::DirRemove(entry.path().to_path_buf(), e))?;
            }
        }
        Ok(())
    }
}

fn run_tmc_command(
    path: &Path,
    extra_args: &[&str],
    timeout: Option<Duration>,
) -> Result<std::process::Output, PythonError> {
    let path = path
        .canonicalize()
        .map_err(|e| PythonError::Path(path.to_path_buf(), e))?;
    log::debug!("running tmc command at {}", path.display());
    let common_args = ["-m", "tmc"];

    let (name, mut command) = match &*LOCAL_PY {
        LocalPy::Unix => ("python3", Command::new("python3")),
        LocalPy::Windows => ("py", Command::new("py")),
        //.map_err(|e| PythonError::Command("py", e))?,
        LocalPy::WindowsConda { conda_path } => ("conda", Command::new(conda_path)),
    };
    let command = match &*LOCAL_PY {
        LocalPy::Unix => command
            .args(&common_args)
            .args(extra_args)
            .current_dir(path),
        LocalPy::Windows => command
            .args(&["-3"])
            .args(&common_args)
            .args(extra_args)
            .current_dir(path),
        LocalPy::WindowsConda { .. } => command
            .args(&common_args)
            .args(extra_args)
            .current_dir(path),
    };
    let output = CommandWithTimeout(command)
        .wait_with_timeout(name, timeout)
        .unwrap();

    log::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    Ok(output)
}

fn parse_exercise_description(path: &Path) -> Result<Vec<TestDesc>, PythonError> {
    let mut test_descs = vec![];
    let mut path = path.to_owned();
    path.push(".available_points.json");
    let file = File::open(&path).map_err(|e| PythonError::FileOpen(path.clone(), e))?;
    // TODO: deserialize directly into Vec<TestDesc>?
    let json: HashMap<String, Vec<String>> = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| PythonError::Deserialize(path, e))?;
    for (key, value) in json {
        test_descs.push(TestDesc::new(key, value));
    }
    Ok(test_descs)
}

fn parse_test_result(path: &Path) -> Result<RunResult, PythonError> {
    let mut path = path.to_owned();
    path.push(".tmc_test_results.json");
    let results_file = File::open(&path).map_err(|e| PythonError::FileOpen(path.clone(), e))?;
    let test_results: Vec<PythonTestResult> = serde_json::from_reader(BufReader::new(results_file))
        .map_err(|e| PythonError::Deserialize(path, e))?;
    let test_results: Vec<TestResult> = test_results
        .into_iter()
        .map(PythonTestResult::into_test_result)
        .collect();

    let mut status = RunStatus::Passed;
    for result in &test_results {
        if !result.successful {
            status = RunStatus::TestsFailed;
        }
    }
    Ok(RunResult::new(status, test_results, HashMap::new()))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::{tempdir, TempDir};
    use tmc_langs_framework::{domain::RunStatus, plugin::LanguagePlugin};

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
        for entry in walkdir::WalkDir::new("tests/data/tmc") {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let entry_path: PathBuf = entry.path().components().skip(2).collect();
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

        let temp = copy_test("tests/data/project");
        let plugin = Python3Plugin::new();
        let ex_desc = plugin.scan_exercise(temp.path(), "name".into()).unwrap();
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

        let temp = copy_test("tests/data/project");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_points.TestEverything.test_new"
        );
        assert!(run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert_eq!(run_result.test_results[0].points.len(), 3);
        assert!(run_result.test_results[0].message.is_empty());
        assert!(run_result.test_results[0].exceptions.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        assert!(run_result.logs.is_empty());

        let temp = copy_test("tests/data/failing");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_failing.TestFailing.test_new"
        );
        assert!(!run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert!(run_result.test_results[0].message.starts_with("'a' != 'b'"));
        assert!(run_result.test_results[0].exceptions.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        assert!(run_result.logs.is_empty());

        let temp = copy_test("tests/data/erroring");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(
            run_result.test_results[0].name,
            "test.test_erroring.TestErroring.test_erroring"
        );
        assert!(!run_result.test_results[0].successful);
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

        let correct = plugin.is_exercise_type_correct(Path::new("tests/data"));
        assert!(correct);

        let correct = plugin.is_exercise_type_correct(Path::new("./"));
        assert!(!correct);
    }

    #[test]
    fn clean() {
        init();
        let plugin = Python3Plugin::new();

        let temp = copy_test("tests/data/clean_target");
        let temp_path = temp.path();

        assert!(temp_path.join(".available_points.json").exists());
        assert!(temp_path
            .join("subdirectory/.tmc_test_results.json")
            .exists());
        assert!(temp_path
            .join("subdirectory/__pycache__/cachefile")
            .exists());
        plugin.clean(temp.path()).unwrap();
        assert!(!temp_path.join(".available_points.json").exists());
        assert!(!temp_path
            .join("subdirectory/.tmc_test_results.json")
            .exists());
        assert!(!temp_path
            .join("subdirectory/__pycache__/cachefile")
            .exists());
        assert!(temp_path.join("leave").exists());
    }
}
