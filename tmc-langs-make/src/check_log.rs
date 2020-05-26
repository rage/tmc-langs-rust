//! Struct representing the results of a Check test run.

use serde::Deserialize;
use std::collections::HashMap;
use tmc_langs_framework::domain::{RunResult, RunStatus, TestResult};

#[derive(Debug, Deserialize)]
pub struct CheckLog {
    datetime: String,
    #[serde(rename = "suite")]
    test_suites: Vec<TestSuite>,
    duration: String,
}

impl CheckLog {
    /// Converts the log into a RunResult. The point map should contain a mapping from test.id to a list of points, e.g.
    /// "test_one" => ["1.1", "1.2"].
    pub fn into_run_result(self, mut point_map: HashMap<String, Vec<String>>) -> RunResult {
        let mut status = RunStatus::Passed;
        let mut test_results = vec![];

        for suite in self.test_suites {
            for test in suite.tests {
                let passed = test.result == "success";
                if !passed {
                    status = RunStatus::TestsFailed;
                }

                let points = point_map.remove(&test.id).unwrap_or_default();
                let exceptions = vec![];
                test_results.push(TestResult {
                    name: test.description,
                    passed,
                    points,
                    message: test.message,
                    exceptions,
                });
            }
        }
        RunResult {
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
