//! Contains structs that model data related to exercises.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};

/// A description of an exercise's test case.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct TestResult {
    pub name: String,
    pub successful: bool,
    /// List of points that were received from the exercise from passed tests.
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub exception: Vec<String>,
}

/// A description of an exercise.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
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

/// How long an exercise's tests may run when its `.tmcproject.yml` sets no `tests_timeout_ms`.
/// Covers the whole test command, so a build the plugin does as part of it (C#, Maven's test
/// goal) counts too; Java and make projects are built before the clock starts.
pub const DEFAULT_TESTS_TIMEOUT: Duration = Duration::from_secs(60);

/// Name of the failing test a timed-out run reports; see [`RunResult::timed_out`].
pub const TIMEOUT_TEST_NAME: &str = "Timeout test";

/// The result of running an exercise's test suite against a submission.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct RunResult {
    /// The overall status of a test run.
    pub status: RunStatus,
    /// Whether each test passed and which points were awarded.
    pub test_results: Vec<TestResult>,
    /// Logs from the test run.
    /// The key may be an arbitrary string identifying the type of log.
    pub logs: HashMap<String, String>,
    /// Set when the tests were stopped for running longer than this many milliseconds: the
    /// exercise's `tests_timeout_ms`, else 60000. The status is then `TESTS_FAILED`, with one
    /// failing test named "Timeout test" whose message says so; an infinite loop is the usual
    /// cause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts-rs", ts(optional))]
    pub timed_out_after_ms: Option<u64>,
}

impl RunResult {
    pub fn new(
        status: RunStatus,
        test_results: Vec<TestResult>,
        logs: HashMap<String, String>,
    ) -> Self {
        Self {
            status,
            test_results,
            logs,
            timed_out_after_ms: None,
        }
    }

    /// The result of a test run stopped after `timeout`, keeping the output it had produced.
    pub fn timed_out(timeout: Duration, stdout: String, stderr: String) -> Self {
        let limit = if timeout.subsec_millis() == 0 {
            format!("{} s", timeout.as_secs())
        } else {
            format!("{} ms", timeout.as_millis())
        };
        let mut logs = HashMap::new();
        logs.insert("stdout".to_string(), stdout);
        logs.insert("stderr".to_string(), stderr);
        Self {
            status: RunStatus::TestsFailed,
            test_results: vec![TestResult {
                name: TIMEOUT_TEST_NAME.to_string(),
                successful: false,
                points: vec![],
                message: format!(
                    "The tests did not finish within {limit} and were stopped. \
                     Make sure your code has no infinite loop."
                ),
                exception: vec![],
            }],
            logs,
            timed_out_after_ms: Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        }
    }
}

/// The overall status of a test run.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExercisePackagingConfiguration {
    /// Student folders or files which are copied from submission.
    pub student_file_paths: HashSet<PathBuf>,
    /// Exercise folders or files which are copied from exercise template or clone.
    pub exercise_file_paths: HashSet<PathBuf>,
    /// How long the exercise's tests may run: its `.tmcproject.yml`'s `tests_timeout_ms`, else
    /// [`DEFAULT_TESTS_TIMEOUT`]. Absent only from CLIs older than this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts-rs", ts(optional))]
    pub tests_timeout_ms: Option<u64>,
    /// The language plugin that recognised the exercise: `python3`, `apache-maven`,
    /// `apache-ant`, `csharp`, `make`, `r` or `No-Tests`. Absent only from CLIs older than this
    /// field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts-rs", ts(optional))]
    pub plugin: Option<String>,
}

/// Determines how style errors are handled.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "UPPERCASE")]
pub enum StyleValidationStrategy {
    Fail,
    Warn,
    Disabled,
}

/// A style validation error.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct StyleValidationError {
    pub column: u32,
    pub line: u32,
    pub message: String,
    pub source_name: String,
}

/// The result of a style check.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct StyleValidationResult {
    pub strategy: StyleValidationStrategy,
    pub validation_errors: Option<HashMap<PathBuf, Vec<StyleValidationError>>>,
}
