//! Implementation of LanguagePlugin for Python 3.

mod error;
mod plugin;
mod policy;

pub use error::PythonError;
pub use plugin::Python3Plugin;
pub use policy::Python3StudentFilePolicy;

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use tmc_langs_framework::domain::TestResult;

enum LocalPy {
    Unix,
    Windows,
    WindowsConda { conda_path: String },
    Custom { python_exec: String },
}

lazy_static! {
    // the python command is platform-dependent
    static ref LOCAL_PY: LocalPy = {
        if let Ok(python_exec) = env::var("TMC_LANGS_PYTHON_EXEC") {
            log::debug!("using Python from environment variable TMC_LANGS_PYTHON_EXEC={}", python_exec);
            return LocalPy::Custom { python_exec };
        }

        if cfg!(windows) {
            // Check for Conda
            let conda = env::var("CONDA_PYTHON_EXE");
            if let Ok(conda_path) = conda {
                if PathBuf::from(&conda_path).exists() {
                    log::debug!("detected conda on windows");
                    return LocalPy::WindowsConda { conda_path };
                }
            }
            log::debug!("detected windows");
            LocalPy::Windows
        } else {
            log::debug!("detected unix");
            LocalPy::Unix
        }
    };
}

#[derive(Debug, Deserialize, Serialize)]
struct PythonTestResult {
    pub name: String,
    pub passed: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub backtrace: Vec<String>,
}

impl PythonTestResult {
    fn into_test_result(self) -> TestResult {
        TestResult {
            name: parse_test_name(self.name),
            successful: self.passed,
            message: parse_test_message(self.message),
            points: self.points,
            exception: self.backtrace,
        }
    }
}

fn parse_test_name(test_name: String) -> String {
    let parts: Vec<_> = test_name.split('.').collect();
    if parts.len() == 4 {
        format!("{}: {}", parts[2], parts[3])
    } else {
        test_name
    }
}

fn parse_test_message(test_message: String) -> String {
    const PREFIX_1: &str = "true is not false :";
    const PREFIX_2: &str = "false is not true :";

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
        let parsed = parse_test_name("test.test_new.TestCase.test_new".to_string());
        assert_eq!(parsed, "TestCase: test_new");
        let parsed = parse_test_name("some.other.test".to_string());
        assert_eq!(parsed, "some.other.test");
    }

    #[test]
    fn parses_test_message() {
        let parsed = parse_test_message("True is not False :   !message!    ".to_string());
        assert_eq!(parsed, "!message!");
        let parsed = parse_test_message("some other message".to_string());
        assert_eq!(parsed, "some other message");
        let parsed = parse_test_message("fAlSe Is NoT tRuE :".to_string());
        assert_eq!(parsed, "");
    }
}
