#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Java plugins for ant and maven

#[cfg(target_env = "musl")]
compile_error!("The Java plugin does not work on musl");

mod ant_plugin;
mod ant_policy;
mod error;
mod java_plugin;
mod maven_plugin;
mod maven_policy;

pub use self::{
    ant_plugin::AntPlugin, ant_policy::AntStudentFilePolicy, error::JavaError,
    maven_plugin::MavenPlugin, maven_policy::MavenStudentFilePolicy,
};
use j4rs::{ClasspathEntry, Instance, InvocationArg, Jvm, JvmBuilder, errors::J4RsError};
use serde::Deserialize;
use std::{fmt::Display, path::PathBuf};
use tempfile::TempPath;
use tmc_langs_framework::ExitStatus;
use tmc_langs_util::file_util;

#[cfg(windows)]
const SEPARATOR: &str = ";";
#[cfg(not(windows))]
const SEPARATOR: &str = ":";

// these jars are required for the plugin to function
const TMC_JUNIT_RUNNER_BYTES: &[u8] = include_bytes!("../deps/tmc-junit-runner-0.2.8.jar");
const TMC_CHECKSTYLE_RUNNER_BYTES: &[u8] =
    include_bytes!("../deps/tmc-checkstyle-runner-3.0.3-20200520.064542-3.jar");
const J4RS_BYTES: &[u8] = include_bytes!("../deps/j4rs-0.22.0-jar-with-dependencies.jar");

struct JvmWrapper {
    jvm: Jvm,
    stdout_path: TempPath,
    stderr_path: TempPath,
}

impl JvmWrapper {
    pub fn with<R>(&self, f: impl FnOnce(&Jvm) -> Result<R, J4RsError>) -> Result<R, JavaError> {
        let res = match f(&self.jvm) {
            Ok(res) => res,
            Err(err) => {
                let stdout = file_util::read_file_to_string_lossy(&self.stdout_path)?;
                let stderr = file_util::read_file_to_string_lossy(&self.stderr_path)?;
                // truncate log files, not a big deal if it fails for whatever reason
                let _ = file_util::create_file(&self.stdout_path);
                let _ = file_util::create_file(&self.stderr_path);
                return Err(JavaError::J4rs {
                    stdout: Some(stdout),
                    stderr: Some(stderr),
                    source: err,
                });
            }
        };
        Ok(res)
    }
}

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
            file_util::write_to_file(TMC_JUNIT_RUNNER_BYTES, &junit_path)?;
        }
    } else {
        log::debug!("failed to read tmc junit runner jar, writing");
        file_util::write_to_file(TMC_JUNIT_RUNNER_BYTES, &junit_path)?;
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
            file_util::write_to_file(TMC_CHECKSTYLE_RUNNER_BYTES, &checkstyle_path)?;
        }
    } else {
        log::debug!("failed to read checkstyle runner jar, writing");
        file_util::write_to_file(TMC_CHECKSTYLE_RUNNER_BYTES, &checkstyle_path)?;
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
            file_util::write_to_file(J4RS_BYTES, &j4rs_path)?;
        }
    } else {
        log::debug!("failed to read j4rs jar, writing");
        file_util::write_to_file(J4RS_BYTES, &j4rs_path)?;
    }
    Ok(j4rs_path)
}

/// Initializes the J4RS JVM.
fn instantiate_jvm() -> Result<JvmWrapper, JavaError> {
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

    // j4rs may panic
    let catch = std::panic::catch_unwind(|| -> Result<Jvm, JavaError> {
        let jvm = JvmBuilder::new()
            .with_base_path(
                tmc_dir
                    .to_str()
                    .ok_or_else(|| JavaError::InvalidUtf8Path(tmc_dir.clone()))?,
            )
            .classpath_entry(junit_runner)
            .classpath_entry(checkstyle_runner)
            .skip_setting_native_lib()
            .java_opt(j4rs::JavaOpt::new("-Dfile.encoding=UTF-8"))
            .build()
            .map_err(JavaError::j4rs)?;
        Ok(jvm)
    });
    let jvm = match catch {
        Ok(jvm_result) => jvm_result?,
        Err(jvm_panic) => {
            // try to extract error message from panic, if any
            let error_message = if let Some(string) = jvm_panic.downcast_ref::<&str>() {
                string.to_string()
            } else if let Ok(string) = jvm_panic.downcast::<String>() {
                *string
            } else {
                "J4rs panicked without an error message".to_string()
            };

            return Err(JavaError::J4rsPanic(error_message));
        }
    };

    // redirect output to files
    let stdout_path = file_util::named_temp_file()?.into_temp_path();
    let out = create_print_stream(
        &jvm,
        stdout_path
            .to_str()
            .expect("Temp path shouldn't contain invalid UTF-8"),
    )?;
    jvm.invoke_static("java.lang.System", "setOut", &[InvocationArg::from(out)])
        .map_err(JavaError::j4rs)?;
    let stderr_path = file_util::named_temp_file()?.into_temp_path();
    let err = create_print_stream(
        &jvm,
        stderr_path
            .to_str()
            .expect("Temp path shouldn't contain invalid UTF-8"),
    )?;
    jvm.invoke_static("java.lang.System", "setErr", &[InvocationArg::from(err)])
        .map_err(JavaError::j4rs)?;
    Ok(JvmWrapper {
        jvm,
        stdout_path,
        stderr_path,
    })
}

fn create_print_stream(jvm: &Jvm, path: &str) -> Result<Instance, JavaError> {
    let file = jvm
        .create_instance(
            "java.io.File",
            &[InvocationArg::try_from(path).map_err(JavaError::j4rs)?],
        )
        .map_err(JavaError::j4rs)?;
    jvm.invoke(&file, "createNewFile", InvocationArg::empty())
        .map_err(JavaError::j4rs)?;
    let file_output_stream = jvm
        .create_instance("java.io.FileOutputStream", &[InvocationArg::from(file)])
        .map_err(JavaError::j4rs)?;
    let print_stream = jvm
        .create_instance(
            "java.io.PrintStream",
            &[InvocationArg::from(file_output_stream)],
        )
        .map_err(JavaError::j4rs)?;
    Ok(print_stream)
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
    // unused
    // class_name: String,
    message: Option<String>,
    stack_trace: Vec<StackTrace>,
    // unused
    // cause: Option<Box<CaughtException>>,
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
