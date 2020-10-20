//! Java plugins for ant and maven

mod ant;
mod error;
mod maven;
mod plugin;

pub use ant::{policy::AntStudentFilePolicy, AntPlugin};
pub use error::JavaError;
pub use maven::{policy::MavenStudentFilePolicy, MavenPlugin};

use j4rs::{ClasspathEntry, Jvm, JvmBuilder};
use serde::Deserialize;
use std::fmt::Display;
use std::path::PathBuf;
use tmc_langs_framework::{io::file_util, subprocess::ExitStatus};

#[cfg(windows)]
const SEPARATOR: &str = ";";
#[cfg(not(windows))]
const SEPARATOR: &str = ":";

// these jars are required for the plugin to function
const TMC_JUNIT_RUNNER_BYTES: &[u8] = include_bytes!("../jars/tmc-junit-runner-0.2.8.jar");
const TMC_CHECKSTYLE_RUNNER_BYTES: &[u8] =
    include_bytes!("../jars/tmc-checkstyle-runner-3.0.3-20200520.064542-3.jar");
const J4RS_BYTES: &[u8] = include_bytes!("../jars/j4rs-0.13.0-jar-with-dependencies.jar");

fn tmc_dir() -> Result<PathBuf, JavaError> {
    let home_dir = dirs::cache_dir().ok_or(JavaError::HomeDir)?;
    Ok(home_dir.join("tmc"))
}

/// Returns the tmc-junit-runner path, creating it if it doesn't exist yet.
fn get_junit_runner_path() -> Result<PathBuf, JavaError> {
    let jar_dir = tmc_dir()?;

    let junit_path = jar_dir.join("tmc-junit-runner.jar");
    if let Ok(bytes) = file_util::read_file(&junit_path) {
        if TMC_CHECKSTYLE_RUNNER_BYTES != bytes.as_slice() {
            log::debug!("updating tmc junit runner jar");
            file_util::write_to_file(&mut TMC_JUNIT_RUNNER_BYTES, &junit_path)?;
        }
    } else {
        log::debug!("failed to read tmc junit runner jar, writing");
        file_util::write_to_file(&mut TMC_JUNIT_RUNNER_BYTES, &junit_path)?;
    }
    Ok(junit_path)
}

/// Returns the tmc-checkstyle-runner path, creating it if it doesn't exist yet.
fn get_checkstyle_runner_path() -> Result<PathBuf, JavaError> {
    let jar_dir = tmc_dir()?;

    let checkstyle_path = jar_dir.join("tmc-checkstyle-runner.jar");
    if let Ok(bytes) = file_util::read_file(&checkstyle_path) {
        if TMC_CHECKSTYLE_RUNNER_BYTES != bytes.as_slice() {
            log::debug!("updating checkstyle runner jar");
            file_util::write_to_file(&mut TMC_CHECKSTYLE_RUNNER_BYTES, &checkstyle_path)?;
        }
    } else {
        log::debug!("failed to read checkstyle runner jar, writing");
        file_util::write_to_file(&mut TMC_CHECKSTYLE_RUNNER_BYTES, &checkstyle_path)?;
    }
    Ok(checkstyle_path)
}

/// Returns the j4rs path, creating it if it doesn't exist yet.
fn initialize_jassets() -> Result<PathBuf, JavaError> {
    let jar_dir = tmc_dir()?;
    let jassets_dir = jar_dir.join("jassets");

    let j4rs_path = jassets_dir.join("j4rs.jar");

    if let Ok(bytes) = file_util::read_file(&j4rs_path) {
        if J4RS_BYTES != bytes.as_slice() {
            log::debug!("updating j4rs jar");
            file_util::write_to_file(&mut J4RS_BYTES, &j4rs_path)?;
        }
    } else {
        log::debug!("failed to read j4rs jar, writing");
        file_util::write_to_file(&mut J4RS_BYTES, &j4rs_path)?;
    }
    Ok(j4rs_path)
}

/// Initializes the J4RS JVM.
fn instantiate_jvm() -> Result<Jvm, JavaError> {
    let junit_runner_path = crate::get_junit_runner_path()?;
    log::debug!("junit runner at {}", junit_runner_path.display());
    let junit_runner_path = junit_runner_path
        .to_str()
        .ok_or_else(|| JavaError::InvalidUtf8Path(junit_runner_path.clone()))?;
    let junit_runner = ClasspathEntry::new(junit_runner_path);

    let checkstyle_runner_path = crate::get_checkstyle_runner_path()?;
    log::debug!("checkstyle runner at {}", checkstyle_runner_path.display());
    let checkstyle_runner_path = checkstyle_runner_path
        .to_str()
        .ok_or_else(|| JavaError::InvalidUtf8Path(checkstyle_runner_path.clone()))?;
    let checkstyle_runner = ClasspathEntry::new(checkstyle_runner_path);

    let j4rs_path = crate::initialize_jassets()?;
    log::debug!("initialized jassets at {}", j4rs_path.display());

    let tmc_dir = tmc_dir()?;

    let jvm = JvmBuilder::new()
        .with_base_path(
            tmc_dir
                .to_str()
                .ok_or_else(|| JavaError::InvalidUtf8Path(tmc_dir.clone()))?,
        )
        .classpath_entry(junit_runner)
        .classpath_entry(checkstyle_runner)
        .skip_setting_native_lib()
        .build()?;

    Ok(jvm)
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct TestMethod {
    class_name: String,
    method_name: String,
    points: Vec<String>,
}

#[derive(Debug)]
struct CompileResult {
    pub status_code: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
struct TestRun {
    pub test_results: PathBuf,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    class_name: String,
    method_name: String,
    point_names: Vec<String>,
    status: TestCaseStatus,
    message: Option<String>,
    exception: Option<CaughtException>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaughtException {
    class_name: String,
    message: Option<String>,
    stack_trace: Vec<StackTrace>,
    cause: Option<Box<CaughtException>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
enum TestCaseStatus {
    Passed,
    Failed,
    Running,
    NotStarted,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StackTrace {
    declaring_class: String,
    file_name: Option<String>,
    line_number: i32,
    method_name: String,
}

impl Display for StackTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start = self
            .file_name
            .as_ref()
            .map(|f| format!("{}:{}", f, self.line_number))
            .unwrap_or_else(|| self.line_number.to_string());
        // string either starts with file_name:line_number or line_number

        write!(
            f,
            "{}: {}.{}",
            start, self.declaring_class, self.method_name
        )
    }
}
