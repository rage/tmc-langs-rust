use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tmc_langs_framework::domain::TestResult;

#[derive(Debug, Deserialize, Serialize)]
pub struct PythonTestResult {
    pub name: String,
    pub passed: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub backtrace: Vec<String>,
}

impl PythonTestResult {
    pub fn into_test_result(mut self, failed_points: &HashSet<String>) -> TestResult {
        self.points.retain(|point| !failed_points.contains(point));
        TestResult {
            name: parse_test_name(self.name),
            successful: self.passed,
            message: parse_test_message(self.message),
            points: self.points,
            exception: self.backtrace,
        }
    }
}

// parses a four part test name a.b.c.d into c: d
fn parse_test_name(test_name: String) -> String {
    let parts: Vec<_> = test_name.split('.').collect();
    if parts.len() == 4 {
        format!("{}: {}", parts[2], parts[3])
    } else {
        test_name
    }
}

// removes a true/false is not false/true prefix from a test message
fn parse_test_message(test_message: String) -> String {
    const PREFIX_1: &str = "true is not false :";
    const PREFIX_2: &str = "false is not true :";

    // can't use strip_prefix here because we want to retain the capitalization of the rest of the msg
    let lower = test_message.to_lowercase();
    if lower.starts_with(PREFIX_1) {
        test_message[PREFIX_1.len()..].trim().to_string()
    } else if lower.starts_with(PREFIX_2) {
        test_message[PREFIX_2.len()..].trim().to_string()
    } else {
        test_message
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_test_name() {
        let parsed = parse_test_name("test.test_second.TestCase.test_fourth".to_string());
        assert_eq!(parsed, "TestCase: test_fourth");
        let parsed = parse_test_name("some.other.test".to_string());
        assert_eq!(parsed, "some.other.test");
    }

    #[test]
    fn parses_test_message() {
        let parsed = parse_test_message("True is not False :   !MessagE!    ".to_string());
        assert_eq!(parsed, "!MessagE!");
        let parsed = parse_test_message("Some Other Message".to_string());
        assert_eq!(parsed, "Some Other Message");
        let parsed = parse_test_message("fAlSe Is NoT tRuE :".to_string());
        assert_eq!(parsed, "");
    }
}
