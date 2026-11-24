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
use std::{
    ffi::OsString,
    fmt::Display,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tempfile::TempPath;
use tmc_langs_framework::{ExitStatus, TmcCommand};
use tmc_langs_util::file_util;

/// How long `java -version` may take before the JDK counts as broken.
const JAVA_START_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(windows)]
const SEPARATOR: &str = ";";
#[cfg(not(windows))]
const SEPARATOR: &str = ":";

// these jars are required for the plugin to function
const TMC_JUNIT_RUNNER_BYTES: &[u8] = include_bytes!("../deps/tmc-junit-runner-0.2.8.jar");
const TMC_CHECKSTYLE_RUNNER_BYTES: &[u8] =
    include_bytes!("../deps/tmc-checkstyle-runner-3.0.3-20200520.064542-3.jar");
const J4RS_BYTES: &[u8] = include_bytes!("../deps/j4rs-0.25.2-jar-with-dependencies.jar");

/// The `java` launcher of the JDK whose JVM the plugin itself runs in, which j4rs locates through
/// `JAVA_HOME`: `$JAVA_HOME/bin/java` when it exists, else whichever `java` is on `PATH`.
fn java_executable() -> PathBuf {
    java_executable_in(std::env::var_os("JAVA_HOME"))
}

fn java_executable_in(java_home: Option<OsString>) -> PathBuf {
    if let Some(java_home) = java_home.filter(|home| !home.is_empty()) {
        let launcher =
            Path::new(&java_home)
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
        if launcher.is_file() {
            return launcher;
        }
    }
    PathBuf::from("java")
}

/// System properties that make a test JVM read and write UTF-8 in an English locale, as in the
/// Linux sandbox, instead of the Windows code page and regional settings. Placed before the
/// user's `JVM_OPTIONS` so those still win.
const TEST_JVM_LOCALE_ARGS: [&str; 6] = [
    "-Dfile.encoding=UTF-8",
    "-Dstdout.encoding=UTF-8",
    "-Dstderr.encoding=UTF-8",
    "-Dsun.stdout.encoding=UTF-8",
    "-Duser.language=en",
    "-Duser.country=US",
];

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

/// Directory for the plugin's cached jars and the bundled Maven.
///
/// On Windows it must be pure ASCII: j4rs hands the class path to `JNI_CreateJavaVM`, which reads
/// it in the ANSI code page, so a non-ASCII profile name (`C:\Users\Mäkelä`) breaks every JVM
/// start. Such profiles get a per-profile directory under the public folder instead.
fn tmc_dir() -> Result<PathBuf, JavaError> {
    let cache_dir = dirs::cache_dir().ok_or(JavaError::HomeDir)?;
    let ascii_fallback_root = if cfg!(windows) {
        std::env::var_os("PUBLIC")
            .or_else(|| std::env::var_os("ProgramData"))
            .map(PathBuf::from)
    } else {
        None
    };
    Ok(tmc_dir_in(&cache_dir, ascii_fallback_root.as_deref()))
}

fn tmc_dir_in(cache_dir: &Path, ascii_fallback_root: Option<&Path>) -> PathBuf {
    let dir = cache_dir.join("tmc");
    match ascii_fallback_root {
        Some(root) if !dir.to_string_lossy().is_ascii() => {
            // FNV-1a: stable across Rust versions, so the directory is not abandoned on upgrade.
            let hash = dir
                .to_string_lossy()
                .bytes()
                .fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                });
            root.join(format!("tmc-{hash:016x}"))
        }
        _ => dir,
    }
}

/// Serializes cache writes between threads; the file lock alone is per process on Unix.
static CACHE_WRITE_MUTEX: Mutex<()> = Mutex::new(());

/// Makes the file at `path` contain `bytes`, leaving it untouched when it already does.
///
/// Safe against concurrent CLI runs: the write goes to a temporary file that is renamed into
/// place under a cache-wide lock, so no process sees or truncates a half-written jar. If the
/// rename is refused because a running JVM has the old file open (Windows), the old file stays.
fn ensure_cached_file(path: &Path, bytes: &[u8]) -> Result<(), JavaError> {
    let is_current =
        |path: &Path| file_util::read_file(path).is_ok_and(|existing| existing == bytes);
    if is_current(path) {
        return Ok(());
    }
    let dir = path
        .parent()
        .ok_or_else(|| JavaError::InvalidExercise(path.to_path_buf()))?;
    file_util::create_dir_all(dir)?;

    let _thread_guard = CACHE_WRITE_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut lock = file_util::Lock::file(
        dir.join("cache-files.lock"),
        file_util::LockOptions::WriteCreate,
    )?;
    let _guard = lock.lock()?;
    if is_current(path) {
        return Ok(());
    }

    log::debug!("writing {}", path.display());
    let mut temp = file_util::named_temp_file_in(dir)?;
    temp.write_all(bytes)
        .and_then(|()| temp.flush())
        .map_err(|e| JavaError::JarWrite(path.to_path_buf(), e))?;
    match temp.persist(path) {
        Ok(_) => Ok(()),
        Err(err) if path.exists() => {
            log::warn!(
                "could not replace {}, keeping the existing file: {}",
                path.display(),
                err.error
            );
            Ok(())
        }
        Err(err) => Err(JavaError::JarWrite(path.to_path_buf(), err.error)),
    }
}

/// Returns the tmc-junit-runner path, creating it if it doesn't exist yet.
fn get_junit_runner_path() -> Result<PathBuf, JavaError> {
    let junit_path = tmc_dir()?.join("tmc-junit-runner.jar");
    ensure_cached_file(&junit_path, TMC_JUNIT_RUNNER_BYTES)?;
    Ok(junit_path)
}

/// Returns the tmc-checkstyle-runner path, creating it if it doesn't exist yet.
fn get_checkstyle_runner_path() -> Result<PathBuf, JavaError> {
    let checkstyle_path = tmc_dir()?.join("tmc-checkstyle-runner.jar");
    ensure_cached_file(&checkstyle_path, TMC_CHECKSTYLE_RUNNER_BYTES)?;
    Ok(checkstyle_path)
}

/// Returns the j4rs path, creating it if it doesn't exist yet.
fn initialize_jassets() -> Result<PathBuf, JavaError> {
    let j4rs_path = tmc_dir()?.join("jassets").join("j4rs.jar");
    ensure_cached_file(&j4rs_path, J4RS_BYTES)?;
    Ok(j4rs_path)
}

/// Checks that the JDK j4rs will load starts at all. A JVM that fails to initialise (say, a
/// JDK missing `libzip`) prints why and exits the whole process from inside j4rs, so the CLI
/// could never report it; a separate launcher fails the same way without taking us down.
fn check_java_starts(java: PathBuf) -> Result<(), JavaError> {
    let output = TmcCommand::piped(java.clone())
        .with(|e| e.arg("-version"))
        .output_with_timeout(JAVA_START_TIMEOUT)?;
    if output.status.success() {
        return Ok(());
    }
    Err(JavaError::UnusableJava {
        java,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
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

    check_java_starts(java_executable())?;

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

    // print version for debugging purposes, but do not fail on error
    match jvm.invoke_static(
        "java.lang.System",
        "getProperty",
        &[InvocationArg::try_from("java.version".to_string()).expect("should never fail")],
    ) {
        Ok(version) => match jvm.to_rust::<String>(version).map_err(JavaError::j4rs) {
            Ok(version) => log::info!("Java version: {version}"),
            Err(err) => log::error!("Error while trying to convert Java version: {err}"),
        },
        Err(err) => log::error!("Error while trying to read Java version: {err}"),
    }

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn tmc_dir_stays_in_cache_dir_when_ascii() {
        let fallback = Path::new("C:\\Users\\Public");
        assert_eq!(
            tmc_dir_in(Path::new("/home/ada/.cache"), Some(fallback)),
            PathBuf::from("/home/ada/.cache/tmc")
        );
    }

    #[test]
    fn tmc_dir_moves_to_ascii_root_for_non_ascii_profiles() {
        let root = Path::new("/public");
        let dir = tmc_dir_in(Path::new("/home/Mäkelä/.cache"), Some(root));
        assert!(dir.starts_with(root));
        assert!(dir.to_string_lossy().is_ascii());
        assert_eq!(
            dir,
            tmc_dir_in(Path::new("/home/Mäkelä/.cache"), Some(root))
        );
        assert_ne!(
            dir,
            tmc_dir_in(Path::new("/home/Väisänen/.cache"), Some(root))
        );
        assert_eq!(
            tmc_dir_in(Path::new("/home/Mäkelä/.cache"), None),
            PathBuf::from("/home/Mäkelä/.cache/tmc")
        );
    }

    #[test]
    fn cached_file_is_created_replaced_and_left_alone_when_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("runner.jar");
        ensure_cached_file(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        ensure_cached_file(&path, b"one").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );

        ensure_cached_file(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn concurrent_cache_writes_never_expose_partial_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runner.jar");
        let bytes = vec![7u8; 1 << 20];
        let readers_saw_partial = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| ensure_cached_file(&path, &bytes).unwrap());
            }
            scope.spawn(|| {
                for _ in 0..200 {
                    if let Ok(read) = std::fs::read(&path) {
                        if read != bytes {
                            readers_saw_partial.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            });
        });
        assert!(!readers_saw_partial.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn test_jvm_gets_utf8_and_fixed_locale() {
        assert!(TEST_JVM_LOCALE_ARGS.contains(&"-Dfile.encoding=UTF-8"));
        assert!(TEST_JVM_LOCALE_ARGS.contains(&"-Duser.language=en"));
    }

    #[test]
    fn junit_runner_is_not_compared_with_the_checkstyle_runner() {
        assert_ne!(TMC_JUNIT_RUNNER_BYTES, TMC_CHECKSTYLE_RUNNER_BYTES);
    }

    #[test]
    fn java_executable_prefers_the_java_home_launcher() {
        let java_home = tempfile::tempdir().unwrap();
        let launcher =
            java_home
                .path()
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "").unwrap();

        assert_eq!(java_executable_in(Some(java_home.path().into())), launcher);
    }

    #[cfg(unix)]
    #[test]
    fn a_java_that_does_not_start_reports_its_output() {
        use std::os::unix::fs::PermissionsExt;
        let java_home = tempfile::tempdir().unwrap();
        let java = java_home.path().join("java");
        std::fs::write(
            &java,
            "#!/bin/sh\necho 'Error occurred during initialization of VM'\n\
             echo 'Unable to load zip library: libzip.so' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();

        let message = check_java_starts(java).unwrap_err().to_string();

        assert!(message.contains("does not start"), "{message}");
        assert!(
            message.contains("Error occurred during initialization of VM"),
            "{message}"
        );
        assert!(message.contains("Unable to load zip library"), "{message}");
    }

    #[test]
    fn java_executable_falls_back_to_path() {
        let without_launcher = tempfile::tempdir().unwrap();
        assert_eq!(
            java_executable_in(Some(without_launcher.path().into())),
            PathBuf::from("java")
        );
        assert_eq!(
            java_executable_in(Some(OsString::new())),
            PathBuf::from("java")
        );
        assert_eq!(java_executable_in(None), PathBuf::from("java"));
    }
}
