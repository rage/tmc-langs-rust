//! Custom wrapper for Command that supports timeouts and contains custom error handling.

use crate::{error::CommandError, TmcError};
use std::io::Read;
use std::time::Duration;
use std::{ffi::OsStr, thread::JoinHandle};
use std::{fs::File, io::Write};
pub use subprocess::ExitStatus;
use subprocess::{Exec, PopenError, Redirection};

/// Wrapper around subprocess::Exec
#[must_use]
pub struct TmcCommand {
    exec: Exec,
    stdin: Option<String>,
}

impl TmcCommand {
    /// Creates a new command
    pub fn new(cmd: impl AsRef<OsStr>) -> Self {
        Self {
            exec: Exec::cmd(cmd).env("LANG", "en_US.UTF-8"),
            stdin: None,
        }
    }

    /// Creates a new command with piped stdout/stderr.
    pub fn piped(cmd: impl AsRef<OsStr>) -> Self {
        Self {
            exec: Exec::cmd(cmd)
                .stdout(Redirection::Pipe)
                .stderr(Redirection::Pipe)
                .env("LANG", "en_US.UTF-8"),
            stdin: None,
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
        }
    }

    // executes the given command and collects its output
    fn execute(self, timeout: Option<Duration>, checked: bool) -> Result<Output, TmcError> {
        let cmd = self.exec.to_cmdline_lossy();
        log::info!("executing {}", cmd);

        let Self { exec, stdin } = self;

        // starts executing the command
        let mut popen = exec.popen().map_err(|e| popen_to_tmc_err(cmd.clone(), e))?;
        let stdin_handle = spawn_writer(popen.stdin.take(), stdin);
        let stdout_handle = spawn_reader(popen.stdout.take());
        let stderr_handle = spawn_reader(popen.stderr.take());

        let exit_status = if let Some(timeout) = timeout {
            // timeout set
            let exit_status = popen
                .wait_timeout(timeout)
                .map_err(|e| popen_to_tmc_err(cmd.clone(), e))?;

            match exit_status {
                Some(exit_status) => exit_status,
                None => {
                    // None means that we timed out
                    popen
                        .terminate()
                        .map_err(|e| CommandError::Terminate(cmd.clone(), e))?;
                    let stdout = stdout_handle
                        .join()
                        .expect("the thread should not be able to panic");
                    let stderr = stderr_handle
                        .join()
                        .expect("the thread should not be able to panic");
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
            popen.wait().map_err(|e| popen_to_tmc_err(cmd.clone(), e))?
        };

        log::info!("finished executing {}", cmd);
        stdin_handle
            .join()
            .expect("the thread should not be able to panic");
        let stdout = stdout_handle
            .join()
            .expect("the thread should not be able to panic");
        let stderr = stderr_handle
            .join()
            .expect("the thread should not be able to panic");

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
            status: exit_status,
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
                    log::error!("failed to write data in writer thread: {}", err);
                }
            }
        }
    })
}

// it's assumed the thread will never panic
fn spawn_reader(file: Option<File>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        if let Some(mut file) = file {
            let mut buf = vec![];
            if let Err(err) = file.read_to_end(&mut buf) {
                log::error!("failed to read data in reader thread: {}", err);
            }
            buf
        } else {
            vec![]
        }
    })
}

// convenience function to convert an error while checking for command not found error
fn popen_to_tmc_err(cmd: String, err: PopenError) -> TmcError {
    if let PopenError::IoError(io) = &err {
        if let std::io::ErrorKind::NotFound = io.kind() {
            TmcError::Command(CommandError::NotFound { cmd, source: err })
        } else {
            TmcError::Command(CommandError::FailedToRun(cmd, err))
        }
    } else {
        TmcError::Command(CommandError::Popen(cmd, err))
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[cfg(test)]
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

    #[test]
    fn not_found() {
        let cmd = TmcCommand::piped("nonexistent command");
        assert!(matches!(
            cmd.output(),
            Err(TmcError::Command(CommandError::NotFound { .. }))
        ));
    }
}
