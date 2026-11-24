//! Custom wrapper for Command that supports timeouts and contains custom error handling.

use crate::{
    TmcError,
    error::CommandError,
    process_tree::ProcessTree,
    tool_lookup::{self, ResolvedTool},
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use subprocess::{Exec, Redirection};

/// How long a build or dependency-resolution step (`mvn compile`, `ant compile-test`,
/// `make test`, `dotnet build`) may run. Below the client's own 10-minute process timeout so
/// the user gets a build-specific error instead of a generic one.
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(8 * 60);

/// How long to wait for a command's output pipes to reach EOF once the command has exited or
/// been killed. A descendant that outlived it may hold the pipes open indefinitely.
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_secs(5);

/// How long to wait for a killed command to be reaped.
const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(5);

/// Wrapper around subprocess::Exec
#[must_use]
pub struct TmcCommand {
    exec: Exec,
    stdin: Option<String>,
    program: OsString,
    /// First argument that could not be passed to a batch file; reported when run.
    argument_error: Option<io::Error>,
}

impl TmcCommand {
    /// Creates a new command. A bare program name is resolved to an absolute path first; see
    /// [`tool_lookup::resolve_tool`].
    pub fn new(cmd: impl Into<OsString>) -> Self {
        let program = cmd.into();
        let exec = match tool_lookup::resolve_tool(&program) {
            Some(ResolvedTool { path, child_path }) => {
                let exec = Exec::cmd(path);
                match child_path {
                    Some(child_path) => exec.env("PATH", child_path),
                    None => exec,
                }
            }
            None => Exec::cmd(&program),
        };
        // Stops cmd.exe (running batch files) from searching the current directory for programs.
        let exec = exec
            .env("LANG", "en_US.UTF-8")
            .env("NoDefaultCurrentDirectoryInExePath", "1");
        Self {
            exec,
            stdin: None,
            program,
            argument_error: None,
        }
    }

    /// Creates a new command with piped stdout/stderr.
    pub fn piped(cmd: impl Into<OsString>) -> Self {
        let command = Self::new(cmd);
        Self {
            exec: command
                .exec
                .stdout(Redirection::Pipe)
                .stderr(Redirection::Pipe),
            ..command
        }
    }

    /// Creates a piped command for a batch file such as `mvn.cmd`, passing `args` so that
    /// `cmd.exe` takes them literally; plain arguments elsewhere.
    pub fn piped_batch_file<A: AsRef<OsStr>>(
        script: impl Into<OsString>,
        args: impl IntoIterator<Item = A>,
    ) -> Self {
        let script = script.into();
        let args: Vec<A> = args.into_iter().collect();
        #[cfg(windows)]
        {
            use subprocess::ExecExt;
            let mut command = Self::piped(
                std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")),
            );
            let mut line = vec![&script as &OsStr];
            line.extend(args.iter().map(AsRef::as_ref));
            match batch_command_line(line) {
                Ok(line) => {
                    command.exec = command
                        .exec
                        .args(["/d", "/s", "/c"])
                        .raw_arg(format!("\"{line}\""));
                }
                Err(err) => command.argument_error = Some(err),
            }
            Self {
                program: script,
                ..command
            }
        }
        #[cfg(not(windows))]
        {
            Self::piped(script).with(|e| e.args(args.iter().map(AsRef::as_ref)))
        }
    }

    /// Allows modification of the internal command without providing access to it.
    pub fn with(self, f: impl FnOnce(Exec) -> Exec) -> Self {
        Self {
            exec: f(self.exec),
            ..self
        }
    }

    /// Gives the command data to write into stdin.
    pub fn set_stdin_data(self, data: String) -> Self {
        Self {
            exec: self.exec.stdin(Redirection::Pipe),
            stdin: Some(data),
            ..self
        }
    }

    // executes the given command and collects its output
    fn execute(self, timeout: Option<Duration>, checked: bool) -> Result<Output, TmcError> {
        let cmd = self.exec.to_cmdline_lossy();
        log::info!("executing {cmd}");

        let Self {
            exec,
            stdin,
            program,
            argument_error,
        } = self;
        if let Some(err) = argument_error {
            return Err(CommandError::FailedToRun(cmd, err).into());
        }

        // Only a command that may be killed needs a group of its own: it keeps a terminal-less
        // child from being stopped by a background read of the controlling terminal.
        #[cfg(unix)]
        let exec = if timeout.is_some() {
            subprocess::ExecExt::setpgid(exec)
        } else {
            exec
        };

        // starts executing the command
        let mut popen = exec
            .start()
            .map_err(|e| popen_to_tmc_err(cmd.clone(), &program, e))?;
        let tree = ProcessTree::attach(popen.pid());
        let stdin_handle = spawn_writer(popen.stdin.take(), stdin);
        let stdout_reader = spawn_reader(popen.stdout.take());
        let stderr_reader = spawn_reader(popen.stderr.take());

        let exit_status = if let Some(timeout) = timeout {
            // timeout set
            let exit_status = popen
                .wait_timeout(timeout)
                .map_err(|e| popen_to_tmc_err(cmd.clone(), &program, e))?;

            match exit_status {
                Some(exit_status) => exit_status,
                None => {
                    // None means that we timed out
                    kill_process_tree(&popen, &tree)
                        .map_err(|e| CommandError::Terminate(cmd.clone(), e))?;
                    let _ = popen.wait_timeout(KILL_REAP_TIMEOUT);
                    let (stdout, stderr) = collect_output(stdout_reader, stderr_reader);
                    return Err(TmcError::Command(CommandError::TimeOut {
                        command: cmd,
                        timeout,
                        stdout: String::from_utf8_lossy(&stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    }));
                }
            }
        } else {
            // no timeout, block until done
            popen
                .wait()
                .map_err(|e| popen_to_tmc_err(cmd.clone(), &program, e))?
        };

        log::info!("finished executing {cmd}");
        stdin_handle
            .join()
            .expect("the thread should not be able to panic");
        let (stdout, stderr) = collect_output(stdout_reader, stderr_reader);

        // on success, log stdout trace and stderr debug
        // on failure if checked, log warn
        // on failure if not checked, log debug
        if !exit_status.success() {
            // if checked is set, error with failed exit status
            if checked {
                log::warn!("stdout: {}", String::from_utf8_lossy(&stdout).into_owned());
                log::warn!("stderr: {}", String::from_utf8_lossy(&stderr).into_owned());
                return Err(CommandError::Failed {
                    command: cmd,
                    status: exit_status,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                }
                .into());
            } else {
                log::debug!("stdout: {}", String::from_utf8_lossy(&stdout).into_owned());
                log::debug!("stderr: {}", String::from_utf8_lossy(&stderr).into_owned());
            }
        } else {
            log::trace!("stdout: {}", String::from_utf8_lossy(&stdout).into_owned());
            log::debug!("stderr: {}", String::from_utf8_lossy(&stderr).into_owned());
        }
        Ok(Output {
            status: exit_status.into(),
            stdout,
            stderr,
        })
    }

    /// Executes the command and waits for its output.
    pub fn status(self) -> Result<ExitStatus, TmcError> {
        self.execute(None, false).map(|o| o.status)
    }

    /// Executes the command and waits for its output.
    pub fn output(self) -> Result<Output, TmcError> {
        self.execute(None, false)
    }

    /// Executes the command and waits for its output and errors if the status is not successful.
    pub fn output_checked(self) -> Result<Output, TmcError> {
        self.execute(None, true)
    }

    /// Executes the command and waits for its output with the given timeout.
    pub fn output_with_timeout(self, timeout: Duration) -> Result<Output, TmcError> {
        self.execute(Some(timeout), false)
    }

    /// Executes the command and waits for its output with the given timeout and errors if the status is not successful.
    pub fn output_with_timeout_checked(self, timeout: Duration) -> Result<Output, TmcError> {
        self.execute(Some(timeout), true)
    }
}

// it's assumed the thread will never panic
fn spawn_writer(file: Option<File>, data: Option<String>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        if let Some(mut file) = file {
            if let Some(data) = data {
                log::debug!("writing data");
                if let Err(err) = file.write_all(data.as_bytes()) {
                    log::error!("failed to write data in writer thread: {err}");
                }
            }
        }
    })
}

/// Output read from a pipe on a background thread, so that waiting for it can be bounded.
struct PipeReader {
    buffer: Arc<Mutex<Vec<u8>>>,
    finished: mpsc::Receiver<()>,
}

// it's assumed the thread will never panic
fn spawn_reader(file: Option<File>) -> PipeReader {
    let buffer = Arc::new(Mutex::new(vec![]));
    let (finished_tx, finished) = mpsc::channel();
    let thread_buffer = Arc::clone(&buffer);
    std::thread::spawn(move || {
        if let Some(mut file) = file {
            let mut chunk = [0u8; 8192];
            loop {
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => thread_buffer
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .extend_from_slice(&chunk[..n]),
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        log::error!("failed to read data in reader thread: {err}");
                        break;
                    }
                }
            }
        }
        let _ = finished_tx.send(());
    });
    PipeReader { buffer, finished }
}

impl PipeReader {
    /// Everything read so far once the pipe closes, or at `deadline` if something still holds
    /// it open.
    fn collect(self, deadline: Instant) -> Vec<u8> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if self.finished.recv_timeout(remaining).is_err() {
            log::warn!("output pipe still open after the command ended");
        }
        std::mem::take(
            &mut *self
                .buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

/// The output of an ended command, waiting at most [`OUTPUT_DRAIN_GRACE`] for both pipes.
fn collect_output(stdout: PipeReader, stderr: PipeReader) -> (Vec<u8>, Vec<u8>) {
    let deadline = Instant::now() + OUTPUT_DRAIN_GRACE;
    (stdout.collect(deadline), stderr.collect(deadline))
}

/// Kills the command and everything it started.
fn kill_process_tree(popen: &subprocess::Job, tree: &ProcessTree) -> io::Result<()> {
    tree.kill();
    #[cfg(unix)]
    {
        use subprocess::unix::JobExt;
        const SIGKILL: i32 = 9;
        if popen.send_signal_group(SIGKILL).is_ok() {
            return Ok(());
        }
    }
    popen.kill()
}

/// `ERROR_ACCESS_DENIED`
const WINDOWS_ACCESS_DENIED: i32 = 5;
/// `ERROR_VIRUS_INFECTED` and `ERROR_VIRUS_DELETED`
const WINDOWS_VIRUS_ERRORS: [i32; 2] = [225, 226];

/// Whether a failure to start `program` looks like antivirus software blocked or removed it:
/// the OS said so, or an executable named by path is gone or denied right after being built.
fn is_blocked_executable(program: &OsStr, err: &io::Error) -> bool {
    if err
        .raw_os_error()
        .is_some_and(|code| WINDOWS_VIRUS_ERRORS.contains(&code))
    {
        return true;
    }
    let is_path = Path::new(program).components().count() > 1;
    match err.kind() {
        io::ErrorKind::NotFound => is_path,
        _ => cfg!(windows) && err.raw_os_error() == Some(WINDOWS_ACCESS_DENIED),
    }
}

/// Whether a tool's output carries Windows' `ERROR_VIRUS_INFECTED` message, which `make` and
/// shells relay when they cannot start a program that antivirus software blocked.
pub fn output_shows_blocked_executable(output: &str) -> bool {
    output
        .to_lowercase()
        .contains("contains a virus or potentially unwanted software")
}

// convenience function to convert an error while checking for command not found error
fn popen_to_tmc_err(cmd: String, program: &OsStr, err: io::Error) -> TmcError {
    if is_blocked_executable(program, &err) {
        return TmcError::Command(CommandError::ExecutableBlocked {
            command: cmd,
            source: err,
        });
    }
    if let std::io::ErrorKind::NotFound = err.kind() {
        let tool = Path::new(program)
            .file_name()
            .unwrap_or(program)
            .to_string_lossy()
            .into_owned();
        let hint = tool_lookup::missing_tool_hint(&tool);
        TmcError::Command(CommandError::NotFound {
            cmd,
            tool,
            hint,
            source: err,
        })
    } else {
        TmcError::Command(CommandError::FailedToRun(cmd, err))
    }
}

/// `args` as the single command line `cmd.exe /d /s /c "<line>"` should receive, each argument
/// quoted so that `cmd.exe` metacharacters (`& | ^ < > ( )`) stay literal.
pub fn batch_command_line<'a>(args: impl IntoIterator<Item = &'a OsStr>) -> io::Result<String> {
    let mut line = String::new();
    for (i, arg) in args.into_iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&quote_batch_arg(arg)?);
    }
    Ok(line)
}

/// Always quoted. `"` is doubled and `%` is broken up (`%%cd:~,%` expands to `%`) the way Rust's
/// standard library does for `.bat`/`.cmd` files; line breaks and NULs cannot be quoted.
fn quote_batch_arg(arg: &OsStr) -> io::Result<String> {
    let arg = arg.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "argument is not valid UTF-8")
    })?;
    if arg.contains(['\0', '\r', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "argument contains a NUL or line break",
        ));
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
                continue;
            }
            '"' => {
                // MSVCRT: backslashes before a quote are doubled and the quote is escaped;
                // cmd.exe additionally needs the quote doubled.
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push_str("\"\"");
            }
            '%' => quoted.push_str("%%cd:~,%"),
            c => quoted.push(c),
        }
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes));
    quoted.push('"');
    Ok(quoted)
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub struct ExitStatus {
    /// `None` if killed by a signal or undetermined — both treated as failure.
    code: Option<i32>,
}

impl ExitStatus {
    /// For a process that exited normally with `code`.
    pub fn new(code: i32) -> Self {
        Self { code: Some(code) }
    }

    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

impl From<subprocess::ExitStatus> for ExitStatus {
    fn from(value: subprocess::ExitStatus) -> Self {
        // Not `signal()`: it is `None` on a normal exit, so every success would read as failure.
        Self {
            code: value.code().map(|c| c as i32),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn timeout() {
        let cmd = TmcCommand::piped("sleep").with(|e| e.arg("2"));
        assert!(matches!(
            cmd.output_with_timeout(Duration::from_nanos(1)),
            Err(TmcError::Command(CommandError::TimeOut { .. }))
        ));
    }

    fn shell_exit(code: i32) -> ExitStatus {
        // Separate args so no quoting is involved: cmd.exe sees `/c exit <code>`.
        #[cfg(windows)]
        let cmd = TmcCommand::piped("cmd").with(|e| e.arg("/c").arg("exit").arg(code.to_string()));
        #[cfg(not(windows))]
        let cmd = TmcCommand::piped("sh").with(|e| e.arg("-c").arg(format!("exit {code}")));
        cmd.status().expect("the shell should be runnable")
    }

    #[test]
    fn success_reflects_exit_code() {
        // Guards against deriving success from `signal()`, which is `None` on normal exit.
        assert!(shell_exit(0).success());
        assert!(!shell_exit(3).success());
    }

    #[test]
    fn not_found_names_the_tool_and_a_remedy() {
        let Err(TmcError::Command(err @ CommandError::NotFound { .. })) =
            TmcCommand::piped("tmc-no-such-tool").output()
        else {
            panic!("expected NotFound");
        };
        let message = err.to_string();
        assert!(message.contains("`tmc-no-such-tool`"), "{message}");
        assert!(message.contains("close all VS Code windows"), "{message}");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_grandchildren_and_returns_promptly() {
        // The shell prints the pid of a background sleep that holds the output pipes.
        let cmd = TmcCommand::piped("sh").with(|e| e.arg("-c").arg("sleep 60 & echo $!; wait"));
        let started = std::time::Instant::now();
        let Err(TmcError::Command(CommandError::TimeOut { stdout, .. })) =
            cmd.output_with_timeout(Duration::from_millis(500))
        else {
            panic!("expected a timeout");
        };
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid: u32 = stdout.trim().parse().unwrap();
        // The group kill is SIGKILL; allow the zombie to be reaped by init.
        let mut is_alive = true;
        for _ in 0..50 {
            is_alive = std::path::Path::new(&format!("/proc/{pid}")).exists()
                && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .map(|stat| stat.contains(") Z "))
                    .unwrap_or(true);
            if !is_alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!is_alive, "grandchild {pid} survived the timeout");
    }

    #[cfg(unix)]
    #[test]
    fn output_wait_is_bounded_when_a_descendant_keeps_the_pipe() {
        // The shell exits at once; its background sleep keeps stdout open.
        let cmd = TmcCommand::piped("sh").with(|e| e.arg("-c").arg("sleep 20 & echo started"));
        let started = std::time::Instant::now();
        let output = cmd.output().unwrap();
        assert!(started.elapsed() < OUTPUT_DRAIN_GRACE + Duration::from_secs(3));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "started");
    }

    #[test]
    fn quotes_batch_arguments_for_cmd() {
        let line = |args: &[&str]| batch_command_line(args.iter().map(OsStr::new)).unwrap();
        assert_eq!(line(&["mvn.cmd", "clean"]), r#""mvn.cmd" "clean""#);
        assert_eq!(
            line(&["-Dout=C:\\tmp\\a&b (1)\\x^y"]),
            r#""-Dout=C:\tmp\a&b (1)\x^y""#
        );
        assert_eq!(line(&["50%"]), r#""50%%cd:~,%""#);
        assert_eq!(line(&[r#"say "hi""#]), r#""say ""hi""""#);
        assert_eq!(line(&["dir\\"]), r#""dir\\""#);
        assert_eq!(line(&[""]), r#""""#);
        assert!(batch_command_line([OsStr::new("a\nb")]).is_err());
        assert!(batch_command_line([OsStr::new("a\0b")]).is_err());
    }

    #[test]
    fn recognizes_the_windows_virus_message() {
        assert!(output_shows_blocked_executable(
            "make: ./test: Operation did not complete successfully because the file contains a virus or potentially unwanted software."
        ));
        assert!(!output_shows_blocked_executable(
            "make: *** [Makefile:3: test] Error 1"
        ));
    }

    #[test]
    fn blocked_executable_classification() {
        let virus = io::Error::from_raw_os_error(225);
        assert!(is_blocked_executable(OsStr::new("make"), &virus));
        let gone = io::Error::from(io::ErrorKind::NotFound);
        assert!(is_blocked_executable(OsStr::new("/build/test"), &gone));
        assert!(!is_blocked_executable(OsStr::new("make"), &gone));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(!is_blocked_executable(OsStr::new("/build/test"), &denied));
        let message =
            popen_to_tmc_err("/build/test".into(), OsStr::new("/build/test"), gone).to_string();
        assert!(message.contains("Failed to run command"), "{message}");
    }

    #[test]
    fn not_found() {
        let cmd = TmcCommand::piped("nonexistent command");
        assert!(matches!(
            cmd.output(),
            Err(TmcError::Command(CommandError::NotFound { .. }))
        ));
    }
}
