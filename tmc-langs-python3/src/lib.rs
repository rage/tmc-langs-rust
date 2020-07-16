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
            name: self.name,
            successful: self.passed,
            message: self.message,
            points: self.points,
            exception: self.backtrace,
        }
    }
}
