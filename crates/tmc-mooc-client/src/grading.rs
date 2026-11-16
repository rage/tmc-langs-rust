//! Per-test results read out of a grading's `feedback_json`.
//!
//! The host treats `feedback_json` as opaque, but the only service the client API serves is the
//! tmc plugin, which stores the test run's tmc-langs `RunResult` there. Mirrored here rather than
//! depending on `tmc-langs-framework`, like [`crate::PublicSpec`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(feature = "ts-rs")]
use ts_rs::TS;

/// The test run a grading was based on.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct GradingTestRun {
    pub status: GradingTestRunStatus,
    /// Empty when the tests did not run, e.g. on a compile failure.
    pub test_results: Vec<GradingTestResult>,
    /// The run's output keyed by log type (e.g. `stdout`, `stderr`). Empty unless `status` is
    /// `COMPILE_FAILED` or `GENERIC_ERROR`, where it holds the compiler or error output.
    pub logs: HashMap<String, String>,
}

/// The overall status of a test run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum GradingTestRunStatus {
    Passed,
    TestsFailed,
    CompileFailed,
    TestrunInterrupted,
    GenericError,
}

/// The result of a single test case.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct GradingTestResult {
    pub name: String,
    pub successful: bool,
    /// The points the test earned; empty when it failed.
    #[serde(default)]
    pub points: Vec<String>,
    #[serde(default)]
    pub message: String,
    /// Stack trace lines of a failure.
    #[serde(default)]
    pub exception: Vec<String>,
}

/// tmc-langs' `RunResult` as the tmc plugin stores it. The defaults admit the plugin's own
/// `{"status": "GENERIC_ERROR", ...}` payloads, which carry neither list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRunResult {
    status: GradingTestRunStatus,
    #[serde(default)]
    test_results: Vec<GradingTestResult>,
    #[serde(default)]
    logs: HashMap<String, String>,
}

impl GradingTestRun {
    /// Reads a grading's `feedback_json`. `None` for anything that is not a test run, since a
    /// service's blob must never fail the grading it came with.
    pub fn from_feedback_json(feedback_json: serde_json::Value) -> Option<Self> {
        let stored = serde_json::from_value::<StoredRunResult>(feedback_json).ok()?;
        // Logs also hold the program's output on a normal run; only an error's logs explain it.
        let logs = match stored.status {
            GradingTestRunStatus::CompileFailed | GradingTestRunStatus::GenericError => stored.logs,
            _ => HashMap::new(),
        };
        Some(Self {
            status: stored.status,
            test_results: stored.test_results,
            logs,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_run_result() {
        let run = GradingTestRun::from_feedback_json(json!({
            "status": "TESTS_FAILED",
            "testResults": [
                {
                    "name": "Tests passes",
                    "successful": true,
                    "points": ["1.1"],
                    "message": "",
                    "exception": []
                },
                {
                    "name": "Tests fails",
                    "successful": false,
                    "points": ["1.2"],
                    "message": "expected 1, got 2",
                    "exception": ["at Test.fails(Test.java:10)"]
                }
            ],
            "logs": { "stdout": "program output" }
        }))
        .unwrap();

        assert_eq!(run.status, GradingTestRunStatus::TestsFailed);
        assert_eq!(run.test_results.len(), 2);
        assert!(run.test_results[0].successful);
        assert_eq!(run.test_results[1].message, "expected 1, got 2");
        assert_eq!(
            run.test_results[1].exception,
            ["at Test.fails(Test.java:10)"]
        );
        assert!(run.logs.is_empty());
    }

    #[test]
    fn keeps_logs_of_a_compile_failure() {
        let run = GradingTestRun::from_feedback_json(json!({
            "status": "COMPILE_FAILED",
            "testResults": [],
            "logs": { "stdout": "Main.java:3: error: ';' expected" }
        }))
        .unwrap();

        assert_eq!(run.status, GradingTestRunStatus::CompileFailed);
        assert_eq!(run.logs["stdout"], "Main.java:3: error: ';' expected");
    }

    #[test]
    fn reads_the_plugins_generic_error_payloads() {
        for feedback in [
            json!({ "status": "GENERIC_ERROR", "exitCode": 137 }),
            json!({ "status": "GENERIC_ERROR", "error": "INVALID_TEST_OUTPUT_JSON" }),
        ] {
            let run = GradingTestRun::from_feedback_json(feedback).unwrap();
            assert_eq!(run.status, GradingTestRunStatus::GenericError);
            assert!(run.test_results.is_empty());
            assert!(run.logs.is_empty());
        }
    }

    #[test]
    fn gives_up_on_anything_else() {
        for feedback in [
            serde_json::Value::Null,
            json!({ "mock_feedback": "ok" }),
            json!({ "status": "SOMETHING_NEW", "testResults": [] }),
            json!({ "status": "PASSED", "testResults": [{ "name": "no successful" }] }),
            json!("Tests passed"),
        ] {
            assert!(
                GradingTestRun::from_feedback_json(feedback.clone()).is_none(),
                "{feedback}"
            );
        }
    }

    #[test]
    fn serializes_status_like_run_result() {
        let value = serde_json::to_value(GradingTestRunStatus::CompileFailed).unwrap();
        assert_eq!(value, json!("COMPILE_FAILED"));
    }
}
