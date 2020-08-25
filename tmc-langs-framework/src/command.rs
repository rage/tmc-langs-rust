//! Custom wrapper for Command that supports timeouts and contains custom error handling.

use crate::{error::CommandError, TmcError};
use std::fmt;
use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// todo: collect args?
pub struct TmcCommand {
    name: String,
    path: PathBuf,
    command: Command,
}

/// Textual representation of a command, e.g. "ls" "-a"
#[derive(Debug)]
pub struct CommandString(String);

impl fmt::Display for CommandString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TmcCommand {
    pub fn new(name: String) -> Self {
        let path = PathBuf::from(&name);
        Self {
            command: Command::new(&path),
            name,
            path,
        }
    }

    pub fn into_inner(self) -> Command {
        self.command
    }

    pub fn to_string(&self) -> CommandString {
        CommandString(format!("{:?}", self.command))
    }

    pub fn named<P: Into<PathBuf>>(name: impl ToString, path: P) -> Self {
        let path = path.into();
        Self {
            command: Command::new(&path),
            name: name.to_string(),
            path,
        }
    }

    // shadows command's status
    pub fn status(&mut self) -> Result<ExitStatus, TmcError> {
        self.deref_mut().status().map_err(|e| {
            if let std::io::ErrorKind::NotFound = e.kind() {
                TmcError::Command(CommandError::NotFound {
                    name: self.name.clone(),
                    path: self.path.clone(),
                    source: e,
                })
            } else {
                TmcError::Command(CommandError::FailedToRun(self.to_string(), e))
            }
        })
    }

    // shadows command's output
    pub fn output(&mut self) -> Result<Output, TmcError> {
        self.deref_mut().output().map_err(|e| {
            if let std::io::ErrorKind::NotFound = e.kind() {
                TmcError::Command(CommandError::NotFound {
                    name: self.name.clone(),
                    path: self.path.clone(),
                    source: e,
                })
            } else {
                TmcError::Command(CommandError::FailedToRun(self.to_string(), e))
            }
        })
    }

    // calls output and checks the exit status, returning an error if the exit status is failed
    pub fn output_checked(&mut self) -> Result<Output, TmcError> {
        let output = self.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            log::warn!("stdout: {}", stdout);
            log::warn!("stderr: {}", stderr);
            return Err(CommandError::Failed {
                command: self.to_string(),
                status: output.status,
                stdout: stdout.into_owned(),
                stderr: stderr.into_owned(),
            }
            .into());
        }
        log::trace!("stdout: {}", stdout);
        log::debug!("stderr: {}", stderr);
        Ok(output)
    }

    /// Waits with the given timeout. Sets stdout and stderr in order to capture them after erroring.
    pub fn wait_with_timeout(&mut self, timeout: Duration) -> Result<OutputWithTimeout, TmcError> {
        // spawn process and init timer
        let mut child = self
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TmcError::Command(CommandError::Spawn(self.to_string(), e)))?;
        let timer = Instant::now();

        loop {
            match child.try_wait().map_err(TmcError::Process)? {
                Some(_exit_status) => {
                    // done, get output
                    return child
                        .wait_with_output()
                        .map(OutputWithTimeout::Output)
                        .map_err(|e| {
                            if let std::io::ErrorKind::NotFound = e.kind() {
                                TmcError::Command(CommandError::NotFound {
                                    name: self.name.clone(),
                                    path: self.path.clone(),
                                    source: e,
                                })
                            } else {
                                TmcError::Command(CommandError::FailedToRun(self.to_string(), e))
                            }
                        });
                }
                None => {
                    // still running, check timeout
                    if timer.elapsed() > timeout {
                        log::warn!("command {} timed out", self.name);
                        // todo: cleaner method for killing
                        child.kill().map_err(TmcError::Process)?;

                        let mut stdout = vec![];
                        let mut stderr = vec![];
                        let stdout_handle = child.stdout.as_mut().unwrap();
                        let stderr_handle = child.stderr.as_mut().unwrap();
                        stdout_handle.read_to_end(&mut stdout).unwrap();
                        stderr_handle.read_to_end(&mut stderr).unwrap();
                        return Ok(OutputWithTimeout::Timeout { stdout, stderr });
                    }

                    // TODO: gradually increase sleep duration?
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

impl Deref for TmcCommand {
    type Target = Command;

    fn deref(&self) -> &Self::Target {
        &self.command
    }
}

impl DerefMut for TmcCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.command
    }
}

pub enum OutputWithTimeout {
    Output(Output),
    Timeout { stdout: Vec<u8>, stderr: Vec<u8> },
}

impl OutputWithTimeout {
    pub fn stdout(&self) -> &[u8] {
        match self {
            Self::Output(output) => &output.stdout,
            Self::Timeout { stdout, .. } => &stdout,
        }
    }
    pub fn stderr(&self) -> &[u8] {
        match self {
            Self::Output(output) => &output.stderr,
            Self::Timeout { stderr, .. } => &stderr,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn timeout() {
        let mut cmd = TmcCommand::new("sleep".to_string());
        cmd.arg("1");
        let res = cmd.wait_with_timeout(Duration::from_millis(100)).unwrap();
        if let OutputWithTimeout::Timeout { .. } = res {
        } else {
            panic!("unexpected result");
        }
    }
}
