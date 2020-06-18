//! Language plugin for the R language

mod error;
mod plugin;
mod policy;

pub use plugin::RPlugin;
pub use policy::RStudentFilePolicy;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tmc_langs_framework::domain::{RunResult, RunStatus, TestResult};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RRunResult {
    run_status: RRunStatus,
    backtrace: Vec<String>,
    test_results: Vec<RTestResult>,
}

impl From<RRunResult> for RunResult {
    fn from(r_run_result: RRunResult) -> RunResult {
        let mut logs = HashMap::new();
        if !r_run_result.backtrace.is_empty() {
            logs.insert(
                "compiler_output".to_string(),
                r_run_result
                    .backtrace
                    .into_iter()
                    .map(|s| format!("{}\n", s))
                    .flat_map(|s| s.as_bytes().to_vec())
                    .collect(),
            );
        }
        RunResult {
            status: r_run_result.run_status.into(),
            test_results: r_run_result
                .test_results
                .into_iter()
                .map(|t| t.into())
                .collect(),
            logs,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RRunStatus {
    Success,
    RunFailed,
    SourcingFailed,
}

impl From<RRunStatus> for RunStatus {
    fn from(r_run_status: RRunStatus) -> RunStatus {
        match r_run_status {
            RRunStatus::Success => RunStatus::Passed,
            RRunStatus::RunFailed => RunStatus::TestsFailed,
            RRunStatus::SourcingFailed => RunStatus::GenericError,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
struct RTestResult {
    status: RTestStatus,
    name: String,
    message: String,
    backtrace: Vec<String>,
    points: Vec<String>,
}

impl From<RTestResult> for TestResult {
    fn from(r_test_result: RTestResult) -> TestResult {
        TestResult {
            name: r_test_result.name,
            successful: r_test_result.status == RTestStatus::Pass,
            points: r_test_result.points,
            message: r_test_result.message,
            exceptions: r_test_result.backtrace,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RTestStatus {
    Pass,
    Fail,
}
