use serde::Deserialize;
use std::collections::HashMap;
use tmc_langs_framework::domain::{RunResult, RunStatus, TestResult};

#[derive(Debug, Deserialize)]
pub struct CheckLog {
    #[serde(rename = "suite")]
    test_suites: Vec<TestSuite>,
}

impl From<CheckLog> for RunResult {
    fn from(log: CheckLog) -> Self {
        let mut status = RunStatus::Passed;
        let mut test_results = vec![];

        for suite in log.test_suites {
            for test in suite.tests {
                if test.result != "success" {
                    status = RunStatus::TestsFailed;
                }
                test_results.push(TestResult {
                    name: test.description,
                    passed: test.result == "success",
                    points: todo!(),
                    message: test.message,
                    exceptions: todo!(),
                });
            }
        }
        Self {
            status,
            test_results,
            logs: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TestSuite {
    title: String,
    #[serde(rename = "test")]
    tests: Vec<Test>,
    duration: String,
}

#[derive(Debug, Deserialize)]
pub struct Test {
    result: String,
    path: String,
    #[serde(rename = "fn")]
    function: String,
    id: String,
    iteration: String,
    duration: String,
    description: String,
    message: String,
}
