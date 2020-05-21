pub mod ant;
pub mod error;
pub mod maven;
pub mod plugin;

use serde::Deserialize;
use std::path::PathBuf;
use std::process::ExitStatus;
use tmc_langs_framework::Error;

#[cfg(windows)]
const SEPARATOR: &str = ";";
#[cfg(not(windows))]
const SEPARATOR: &str = ":";

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TestMethod {
    class_name: String,
    method_name: String,
    points: Vec<String>,
}

#[derive(Debug)]
pub struct CompileResult {
    pub status_code: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub struct TestRun {
    pub test_results: PathBuf,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    class_name: String,
    method_name: String,
    point_names: Vec<String>,
    status: TestCaseStatus,
    message: Option<String>,
    exception: Option<CaughtException>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaughtException {
    class_name: String,
    message: String,
    stack_trace: Vec<StackTrace>,
    cause: Option<Box<CaughtException>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum TestCaseStatus {
    Passed,
    Failed,
    Running,
    NotStarted,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTrace {
    declaring_class: String,
    file_name: String,
    line_number: i32,
    method_name: String,
}

impl StackTrace {
    pub fn to_string(&self) -> String {
        todo!()
    }
}
