//! Contains the CSTestResult type that models the C# test runner result.

use serde::Deserialize;
use std::collections::HashSet;
use tmc_langs_framework::TestResult;

/// Test result from the C# test runner.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[allow(clippy::upper_case_acronyms)]
pub struct CSTestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub points: Vec<String>,
    pub error_stack_trace: Vec<String>,
}

impl CSTestResult {
    pub fn into_test_result(mut self, failed_points: &HashSet<String>) -> TestResult {
        self.points.retain(|point| !failed_points.contains(point));
        TestResult {
            name: self.name,
            successful: self.passed,
            message: self.message,
            exception: self.error_stack_trace,
            points: self.points,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn deserializes() {
        let s = r#"
{
    "Name": "n",
    "Passed": true,
    "Message": "m",
    "Points": ["p1", "p2"],
    "ErrorStackTrace": ["e1", "e2"]
}
"#;

        let _cstr: CSTestResult = serde_json::from_str(s).unwrap();
    }
}
