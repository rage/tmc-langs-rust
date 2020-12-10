//! Custom wrapper for Command that supports timeouts and contains custom error handling.

use crate::{
    error::{CommandError, FileIo},
    file_util, TmcError,
};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;
use subprocess::{Exec, ExitStatus, PopenError};

/// Wrapper around subprocess::Exec
#[must_use]
pub struct TmcCommand {
    exec: Exec,
    stdout: Option<File>,
    stderr: Option<File>,
}

fn read_output(
    file_handle: Option<File>,
    exec_output: Option<&mut File>,
) -> Result<Vec<u8>, TmcError> {
    let mut v = vec![];
    if let Some(mut file) = file_handle {
        let _ = file.seek(SeekFrom::Start(0)); // ignore errors
        file.read_to_end(&mut v).map_err(FileIo::Generic)?;
    } else if let Some(file) = exec_output {
        let _ = file.seek(SeekFrom::Start(0)); // ignore errors
        file.read_to_end(&mut v).map_err(FileIo::Generic)?;
    }
    Ok(v)
}

impl TmcCommand {
    /// Creates a new command
    pub fn new(cmd: impl AsRef<OsStr>) -> Self {
        Self {
            exec: Exec::cmd(cmd).env("LANG", "en_US.UTF-8"),
            stdout: None,
            stderr: None,
        }
    }

    /// Creates a new command with stdout and stderr redirected to files
    pub fn new_with_file_io(cmd: impl AsRef<OsStr>) -> Result<Self, TmcError> {
        let stdout = file_util::temp_file()?;
        let stderr = file_util::temp_file()?;
        Ok(Self {
            exec: Exec::cmd(cmd)
                .stdout(stdout.try_clone().map_err(FileIo::FileHandleClone)?)
                .stderr(stderr.try_clone().map_err(FileIo::FileHandleClone)?)
                .env("LANG", "en_US.UTF-8"), // some languages may error on UTF-8 files if the LANG variable is unset or set to some non-UTF-8 value
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
    }

    /// Allows modification of the internal command without providing access to it.
    pub fn with(self, f: impl FnOnce(Exec) -> Exec) -> Self {
        Self {
            exec: f(self.exec),
            ..self
        }
    }

    // executes the given command and collects its output
    fn execute(self, timeout: Option<Duration>, checked: bool) -> Result<Output, TmcError> {
        let cmd = self.exec.to_cmdline_lossy();
        log::info!("executing {}", cmd);

        let Self {
            exec,
            stdout,
            stderr,
        } = self;

        // starts executing the command
        let mut popen = exec.popen().map_err(|e| popen_to_tmc_err(cmd.clone(), e))?;

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
                    let stdout = read_output(stdout, popen.stdout.as_mut())?;
                    let stderr = read_output(stderr, popen.stderr.as_mut())?;
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
        let stdout = read_output(stdout, popen.stdout.as_mut())?;
        let stderr = read_output(stderr, popen.stderr.as_mut())?;

        // on success, log stdout trace and stderr debug
        // on failure if checked, log warn
        // on failure if not checked, log debug
        if !exit_status.success() {
            // if checked is set, error with failed exit status
            if checked {
                log::warn!("stdout: {}", String::from_utf8_lossy(&stdout));
                log::warn!("stderr: {}", String::from_utf8_lossy(&stderr));
                return Err(CommandError::Failed {
                    command: cmd,
                    status: exit_status,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                }
                .into());
            } else {
                log::debug!("stdout: {}", String::from_utf8_lossy(&stdout));
                log::debug!("stderr: {}", String::from_utf8_lossy(&stderr));
            }
        } else {
            log::trace!("stdout: {}", String::from_utf8_lossy(&stdout));
            log::debug!("stderr: {}", String::from_utf8_lossy(&stderr));
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
        let cmd = TmcCommand::new_with_file_io("sleep")
            .unwrap()
            .with(|e| e.arg("1"));
        assert!(matches!(
            cmd.output_with_timeout(Duration::from_nanos(1)),
            Err(TmcError::Command(CommandError::TimeOut {..}))
        ));
    }

    #[test]
    fn not_found() {
        let cmd = TmcCommand::new_with_file_io("nonexistent command").unwrap();
        assert!(matches!(
            cmd.output(),
            Err(TmcError::Command(CommandError::NotFound {..}))
        ));
    }
}
