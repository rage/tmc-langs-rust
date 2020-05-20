//! Contains structs that model data related to exercises.

pub mod meta_syntax;

use super::Result;
use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

/// A description of an exercise's test case.
#[derive(Debug, Deserialize, Serialize)]
pub struct TestDesc {
    /// The full name of the test.
    ///
    /// If the language organises tests into suites or classes, it is customary
    /// to name the test as "class_name.method_name".
    pub name: String,
    /// The list of point names that passing this test may give.
    ///
    /// To obtain a point X, the user must pass all exercises that require point X.
    pub points: Vec<String>,
}

impl TestDesc {
    pub fn new(name: String, points: Vec<String>) -> Self {
        Self { name, points }
    }
}

/// The result of a single test case.
#[derive(Debug, Deserialize, Serialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub exceptions: Vec<String>,
}

/// A description of an exercise.
#[derive(Debug, Deserialize, Serialize)]
pub struct ExerciseDesc {
    /// The name of the exercise to be shown to the user.
    /// Does not necessarily match or even contain the directory name.
    pub name: String,
    /// Descriptions of the tests that will be run for this exercise.
    pub tests: Vec<TestDesc>,
}

impl ExerciseDesc {
    pub fn new(name: String, tests: Vec<TestDesc>) -> Self {
        Self { name, tests }
    }
}

/// The result of running an exercise's test suite against a submission.
#[derive(Debug, Deserialize, Serialize)]
pub struct RunResult {
    /// The overall status of a test run.
    pub status: RunStatus,
    /// Whether each test passed and which points were awarded.
    pub test_results: Vec<TestResult>,
    /// Logs from the test run.
    /// The key may be an arbitrary string identifying the type of log.
    pub logs: HashMap<String, Vec<u8>>,
}

impl RunResult {
    pub fn new(
        status: RunStatus,
        test_results: Vec<TestResult>,
        logs: HashMap<String, Vec<u8>>,
    ) -> Self {
        Self {
            status,
            test_results,
            logs,
        }
    }
}

/// The overall status of a test run.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum RunStatus {
    /// The submission and tests compiled and all tests passed.
    Passed,
    /// The submission and tests compiled but some tests failed.
    TestsFailed,
    /// The submission or tests did not compile.
    // TODO: "The compiler error should be given in {@code logs[SpecialLogs.COMPILER_OUTPUT]}."
    CompileFailed,
    /// The submission compiled but testrun was interrupted.
    TestrunInterrupted,
    /// For when no other status seems suitable, or the language plugin has
    /// suffered an internal error.
    // TODO: "Details should be given in {@code logs[SpecialLogs.GENERIC_ERROR_MESSAGE]}.""
    GenericError,
}

/// Represents configuration based on which submission may be packaged.
#[derive(Debug, Deserialize, Serialize)]
pub struct ExercisePackagingConfiguration {
    /// Student folders or files which are copied from submission.
    pub student_file_paths: HashSet<PathBuf>,
    /// Exercise folders or files which are copied from exercise template or clone.
    pub exercise_file_paths: HashSet<PathBuf>,
}

impl ExercisePackagingConfiguration {
    pub fn new(
        student_file_paths: HashSet<PathBuf>,
        exercise_file_paths: HashSet<PathBuf>,
    ) -> Self {
        Self {
            student_file_paths,
            exercise_file_paths,
        }
    }
}

/// Extra data from a `.tmcproject.yml` file.
#[derive(Debug, Deserialize, Default)]
pub struct TmcProjectYml {
    #[serde(default)]
    pub extra_student_files: Vec<PathBuf>,
    #[serde(default)]
    pub extra_exercise_files: Vec<PathBuf>,
    #[serde(default)]
    pub force_update: Vec<PathBuf>,
}

impl TmcProjectYml {
    pub fn from(project_dir: &Path) -> Result<Self> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");
        if !config_path.exists() {
            debug!("no config");
            return Ok(Self::default());
        }
        debug!("reading .tmcprojectyml from {}", config_path.display());
        let file = File::open(config_path)?;
        Ok(serde_yaml::from_reader(file)?)
    }
}
