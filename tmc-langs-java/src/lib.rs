pub mod ant;
pub mod error;
pub mod maven;
pub mod plugin;

use error::JavaPluginError;

use j4rs::{ClasspathEntry, Jvm, JvmBuilder};
use serde::Deserialize;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitStatus;

#[cfg(windows)]
const SEPARATOR: &str = ";";
#[cfg(not(windows))]
const SEPARATOR: &str = ":";

const TMC_JUNIT_RUNNER_BYTES: &[u8] = include_bytes!("../jars/tmc-junit-runner-0.2.8.jar");
const TMC_CHECKSTYLE_RUNNER_BYTES: &[u8] =
    include_bytes!("../jars/tmc-checkstyle-runner-3.0.3-20200520.064542-3.jar");
const J4RS_BYTES: &[u8] = include_bytes!("../jars/j4rs-0.11.2-jar-with-dependencies.jar");

fn tmc_dir() -> Result<PathBuf, JavaPluginError> {
    let home_dir = dirs::cache_dir().ok_or(JavaPluginError::HomeDir)?;
    Ok(home_dir.join("tmc"))
}

fn get_junit_runner_path() -> Result<PathBuf, JavaPluginError> {
    let jar_dir = tmc_dir()?;

    let junit_path = jar_dir.join("tmc-junit-runner.jar");
    if !junit_path.exists() {
        fs::create_dir_all(&jar_dir).map_err(|e| JavaPluginError::Dir(jar_dir, e))?;
        let mut file =
            File::create(&junit_path).map_err(|e| JavaPluginError::File(junit_path.clone(), e))?;
        file.write_all(TMC_JUNIT_RUNNER_BYTES)
            .map_err(|e| JavaPluginError::File(junit_path.clone(), e))?;
    }
    Ok(junit_path)
}

fn get_checkstyle_runner_path() -> Result<PathBuf, JavaPluginError> {
    let jar_dir = tmc_dir()?;

    let checkstyle_path = jar_dir.join("tmc-checkstyle-runner.jar");
    if !checkstyle_path.exists() {
        fs::create_dir_all(&jar_dir).map_err(|e| JavaPluginError::Dir(jar_dir, e))?;
        let mut file = File::create(&checkstyle_path)
            .map_err(|e| JavaPluginError::File(checkstyle_path.clone(), e))?;
        file.write_all(TMC_CHECKSTYLE_RUNNER_BYTES)
            .map_err(|e| JavaPluginError::File(checkstyle_path.clone(), e))?;
    }
    Ok(checkstyle_path)
}

fn initialize_jassets() -> Result<PathBuf, JavaPluginError> {
    let jar_dir = tmc_dir()?;
    let jassets_dir = jar_dir.join("jassets");

    let j4rs_path = jassets_dir.join("j4rs.jar");
    if !j4rs_path.exists() {
        fs::create_dir_all(&jassets_dir).map_err(|e| JavaPluginError::Dir(jassets_dir, e))?;
        let mut file =
            File::create(&j4rs_path).map_err(|e| JavaPluginError::File(j4rs_path.clone(), e))?;
        file.write_all(J4RS_BYTES)
            .map_err(|e| JavaPluginError::File(j4rs_path.clone(), e))?;
    }
    Ok(j4rs_path)
}

fn instantiate_jvm() -> Result<Jvm, JavaPluginError> {
    let junit_runner_path = crate::get_junit_runner_path()?;
    log::debug!("junit runner at {}", junit_runner_path.display());
    let junit_runner_path = junit_runner_path.to_str().unwrap();
    let junit_runner = ClasspathEntry::new(junit_runner_path);

    let checkstyle_runner_path = crate::get_checkstyle_runner_path()?;
    log::debug!("checkstyle runner at {}", checkstyle_runner_path.display());
    let checkstyle_runner_path = checkstyle_runner_path.to_str().unwrap();
    let checkstyle_runner = ClasspathEntry::new(checkstyle_runner_path);

    let j4rs_path = crate::initialize_jassets()?;
    log::debug!("initialized jassets at {}", j4rs_path.display());

    let tmc_dir = tmc_dir()?;

    let jvm = JvmBuilder::new()
        .with_base_path(tmc_dir.to_str().unwrap())
        .classpath_entry(junit_runner)
        .classpath_entry(checkstyle_runner)
        .build()
        .expect("failed to build jvm");

    Ok(jvm)
}

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
        format!(
            "{}:{}: {}.{}",
            self.file_name, self.line_number, self.declaring_class, self.method_name
        )
    }
}
