//! Contains the CSTestResult type that models the C# test runner result.

use serde::Deserialize;
use tmc_langs_framework::domain::TestResult;

/// Test result from the C# test runner
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CSTestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub points: Vec<String>,
    pub error_stack_trace: Vec<String>,
}

impl From<CSTestResult> for TestResult {
    fn from(test_result: CSTestResult) -> Self {
        TestResult {
            name: test_result.name,
            successful: test_result.passed,
            message: test_result.message,
            exception: test_result.error_stack_trace,
            points: test_result.points,
        }
    }
}

#[cfg(test)]
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
