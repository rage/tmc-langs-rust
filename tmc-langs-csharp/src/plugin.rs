//! Implementation of LanguagePlugin for C#
mod policy;

pub use policy::CSharpStudentFilePolicy;

use crate::{CSTestResult, CSharpError};

use tmc_langs_framework::{
    domain::{
        ExerciseDesc, RunResult, RunStatus, Strategy, TestDesc, TestResult, ValidationResult,
    },
    plugin::Language,
    CommandWithTimeout, LanguagePlugin, OutputWithTimeout, TmcError,
};

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use walkdir::WalkDir;
use zip::ZipArchive;

const TMC_CSHARP_RUNNER: &[u8] = include_bytes!("../tmc-csharp-runner-1.0.1.zip");

#[derive(Default)]
pub struct CSharpPlugin {}

impl CSharpPlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Extracts the included TMC_CSHARP_RUNNER to the given path
    fn extract_runner(target: &Path) -> Result<(), CSharpError> {
        log::debug!("extracting C# runner to {}", target.display());

        let mut zip = ZipArchive::new(Cursor::new(TMC_CSHARP_RUNNER)).map_err(CSharpError::Zip)?;
        for i in 0..zip.len() {
            let file = zip.by_index(i).unwrap();
            if file.is_file() {
                let target_file_path = target.join(file.sanitized_name());
                if let Some(parent) = target_file_path.parent() {
                    fs::create_dir_all(&parent)
                        .map_err(|e| CSharpError::CreateDir(target_file_path.clone(), e))?;
                }
                let mut target_file = File::create(&target_file_path)
                    .map_err(|e| CSharpError::CreateFile(target_file_path.clone(), e))?;
                let bytes: Vec<u8> = file
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| CSharpError::ReadFile(target_file_path.clone(), e))?;
                target_file
                    .write_all(&bytes)
                    .map_err(|e| CSharpError::WriteFile(target_file_path, e))?;
            }
        }
        Ok(())
    }

    /// Returns the directory of the TMC C# runner, writing it to cache if it doesn't exist there yet
    fn get_runner_dir() -> Result<PathBuf, CSharpError> {
        match dirs::cache_dir() {
            Some(cache_dir) => {
                let runner_dir = cache_dir.join("tmc").join("tmc-csharp-runner");
                if !runner_dir.exists() {
                    Self::extract_runner(&runner_dir)?;
                }
                Ok(runner_dir)
            }
            None => Err(CSharpError::CacheDir),
        }
    }

    /// Returns the path to the TMC C# runner in the cache. If TMC_CSHARP_BOOTSTRAP_PATH, it is returned instead.
    fn get_bootstrap_path() -> Result<PathBuf, CSharpError> {
        if let Ok(var) = env::var("TMC_CSHARP_BOOTSTRAP_PATH") {
            log::debug!("using bootstrap path TMC_CSHARP_BOOTSTRAP_PATH={}", var);
            Ok(PathBuf::from(var))
        } else {
            let runner_path = Self::get_runner_dir()?;
            let bootstrap = runner_path.join("TestMyCode.CSharp.Bootstrap.dll");
            if bootstrap.exists() {
                log::debug!("found boostrap dll at {}", bootstrap.display());
                Ok(bootstrap)
            } else {
                Err(CSharpError::MissingBootstrapDll(bootstrap))
            }
        }
    }

    /// Parses the test results JSON file at the path argument.
    fn parse_test_results(test_results_path: &Path) -> Result<RunResult, CSharpError> {
        let test_results = File::open(test_results_path)
            .map_err(|e| CSharpError::ReadFile(test_results_path.to_path_buf(), e))?;
        let test_results: Vec<CSTestResult> = serde_json::from_reader(test_results)
            .map_err(|e| CSharpError::ParseTestResults(test_results_path.to_path_buf(), e))?;

        let mut status = RunStatus::Passed;
        for test_result in &test_results {
            if !test_result.passed {
                status = RunStatus::TestsFailed;
                break;
            }
        }
        let test_results = test_results.into_iter().map(|t| t.into()).collect();
        Ok(RunResult {
            status,
            test_results,
            logs: HashMap::new(),
        })
    }
}

impl LanguagePlugin for CSharpPlugin {
    const PLUGIN_NAME: &'static str = "csharp";
    type StudentFilePolicy = CSharpStudentFilePolicy;

    // checks the directories in src for csproj files
    fn is_exercise_type_correct(path: &Path) -> bool {
        WalkDir::new(path.join("src"))
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension() == Some(&OsString::from("csproj")))
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        Self::StudentFilePolicy::new(project_path.to_path_buf())
    }

    // runs --generate-points-file and parses the generated .tmc_available_points.json
    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        let output = Command::new("dotnet")
            .current_dir(path)
            .arg(Self::get_bootstrap_path()?)
            .arg("--generate-points-file")
            .output()
            .map_err(|e| CSharpError::RunFailed("dotnet", e))?;
        log::trace!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            return Err(CSharpError::CommandFailed("dotnet", output.status).into());
        }

        let exercise_desc_json_path = path.join(".tmc_available_points.json");
        let exercise_desc_json = File::open(&exercise_desc_json_path)
            .map_err(|e| CSharpError::ReadFile(exercise_desc_json_path.clone(), e))?;
        let json: HashMap<String, Vec<String>> =
            serde_json::from_reader(BufReader::new(exercise_desc_json))
                .map_err(|e| CSharpError::ParseExerciseDesc(exercise_desc_json_path, e))?;

        let mut tests = vec![];
        for (key, value) in json {
            tests.push(TestDesc::new(key, value));
        }
        Ok(ExerciseDesc {
            name: exercise_name,
            tests,
        })
    }

    // removes any existing .tmc_test_results.json, runs --run-tests and parses the resulting .tmc_test_results.json
    fn run_tests_with_timeout(
        &self,
        path: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            fs::remove_file(&test_results_path)
                .map_err(|e| CSharpError::RemoveFile(test_results_path.clone(), e))?;
        }
        let output = match CommandWithTimeout(
            Command::new("dotnet")
                .current_dir(path)
                .arg(Self::get_bootstrap_path()?)
                .arg("--run-tests"),
        )
        .wait_with_timeout("dotnet", timeout)
        {
            Ok(output) => output,
            Err(err) => {
                log::error!("Error running dotnet: {}", err);
                return Ok(RunResult {
                    status: RunStatus::GenericError,
                    test_results: vec![],
                    logs: HashMap::new(),
                });
            }
        };
        log::trace!("stdout: {}", String::from_utf8_lossy(output.stdout()));
        log::debug!("stderr: {}", String::from_utf8_lossy(output.stderr()));

        match output {
            OutputWithTimeout::Output(output) => {
                if !output.status.success() {
                    let mut logs = HashMap::new();
                    logs.insert("stdout".to_string(), output.stdout);
                    logs.insert("stderr".to_string(), output.stderr);
                    return Ok(RunResult {
                        status: RunStatus::CompileFailed,
                        test_results: vec![],
                        logs,
                    });
                }
                Self::parse_test_results(&test_results_path).map_err(|e| e.into())
            }
            OutputWithTimeout::Timeout { .. } => Ok(RunResult {
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
                logs: HashMap::new(),
            }),
        }
    }

    // no checkstyle for C#
    fn check_code_style(&self, _path: &Path, _locale: Language) -> Option<ValidationResult> {
        Some(ValidationResult {
            strategy: Strategy::Disabled,
            validation_errors: None,
        })
    }

    // removes all bin and obj sub-directories
    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            fs::remove_file(&test_results_path)
                .map_err(|e| CSharpError::RemoveFile(test_results_path.clone(), e))?;
        }
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let file_name = entry.path().file_name();
            if file_name == Some(&OsString::from("bin"))
                || file_name == Some(&OsString::from("obj"))
            {
                fs::remove_dir_all(entry.path())
                    .map_err(|e| CSharpError::RemoveDir(entry.path().to_path_buf(), e))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Once;
    use tempfile::TempDir;

    static INIT_RUNNER: Once = Once::new();

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
        INIT_RUNNER.call_once(|| {
            let _ = CSharpPlugin::get_runner_dir().unwrap();
        });
    }

    fn copy_test_dir(path: &str) -> TempDir {
        init();
        let path = Path::new(path);

        let temp = tempfile::tempdir().unwrap();
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let target = temp.path().join(entry.path().strip_prefix(path).unwrap());
            if entry.path().is_dir() {
                log::trace!("creating dirs {}", entry.path().display());
                fs::create_dir_all(target).unwrap();
            } else {
                log::trace!(
                    "copy from {} to {}",
                    entry.path().display(),
                    target.display()
                );
                fs::copy(entry.path(), target).unwrap();
            }
        }
        temp
    }

    #[test]
    fn exercise_type_is_correct() {
        init();
        let temp = copy_test_dir("tests/data/PassingProject");
        let is = CSharpPlugin::is_exercise_type_correct(temp.path());
        assert!(is);
    }

    #[test]
    fn exercise_type_is_incorrect() {
        init();
        let temp = copy_test_dir("tests/data");
        let is = CSharpPlugin::is_exercise_type_correct(temp.path());
        assert!(!is);
    }

    #[test]
    fn scans_exercise() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let scan = plugin
            .scan_exercise(temp.path(), "name".to_string())
            .unwrap();
        assert_eq!(scan.name, "name");
        assert_eq!(scan.tests.len(), 6);
    }

    #[test]
    fn runs_tests_passing() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::Passed);
        assert_eq!(res.test_results.len(), 2);
        for tr in res.test_results {
            assert!(tr.successful);
        }
        assert!(res.logs.is_empty());
    }

    #[test]
    fn runs_tests_failing() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/FailingProject");
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
        assert_eq!(res.test_results.len(), 1);
        let test_result = &res.test_results[0];
        assert!(!test_result.successful);
        assert!(test_result.points.is_empty());
        assert!(test_result.message.contains("Expected: False"));
        assert_eq!(test_result.exception.len(), 2);
        assert!(res.logs.is_empty());
    }

    #[test]
    fn runs_tests_compile_err() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/NonCompilingProject");
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::CompileFailed);
        assert!(!res.logs.is_empty());
        assert!(String::from_utf8_lossy(res.logs.get("stdout").unwrap())
            .contains("This is a compile error"));
    }

    #[test]
    fn cleans() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let bin_path = temp.path().join("src").join("PassingSample").join("bin");
        let obj_path_test = temp
            .path()
            .join("test")
            .join("PassingSampleTests")
            .join("obj");
        assert!(!bin_path.exists());
        assert!(!obj_path_test.exists());
        plugin.run_tests(temp.path()).unwrap();
        assert!(bin_path.exists());
        assert!(obj_path_test.exists());
        plugin.clean(temp.path()).unwrap();
        assert!(!bin_path.exists());
        assert!(!obj_path_test.exists());
    }
}
