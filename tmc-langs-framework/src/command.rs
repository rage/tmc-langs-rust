//! Custom wrapper for Command that supports timeouts and contains custom error handling.

use crate::{error::CommandError, TmcError};
use std::{fmt, sync::Mutex};
use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output};
use shared_child::SharedChild;
use std::sync::Arc;
use std::thread;
use std::time::{Duration};
use os_pipe::pipe;
#[cfg(unix)]
use shared_child::unix::SharedChildExt;

// todo: collect args?
#[derive(Debug)]
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
    pub fn wait_with_timeout(mut self, timeout: Duration) -> Result<OutputWithTimeout, TmcError> {
        let name = self.name.clone();
        let path = self.path.clone();
        let self_string = self.to_string();
        let self_string2 = self.to_string();

        let (mut stdout_reader, stdout_writer) = pipe().unwrap();
        let (mut stderr_reader, stderr_writer) = pipe().unwrap();

        let (process_result, timed_out) = {
            let mut command = self.command;
            command.stdout(stdout_writer)
                .stderr(stderr_writer);

            let shared_child = SharedChild::spawn(&mut command)
                .map_err(|e| TmcError::Command(CommandError::Spawn(self_string, e)))?;
            let child_arc = Arc::new(shared_child);

            let running = Arc::new(Mutex::new(true));
            let running_clone = running.clone();
            let timed_out = Arc::new(Mutex::new(false));


            let child_arc_clone = child_arc.clone();
            let timed_out_clone = timed_out.clone();
            let _timeout_checker = thread::spawn(move || {
                thread::sleep(timeout);

                if !running_clone.lock().unwrap().clone() {
                    return;
                }
                let mut timed_out_handle = timed_out_clone.lock().unwrap();
                *timed_out_handle = true;

                #[cfg(unix)]
                {
                    // Ask process to terminate nicely
                    let _res2 = child_arc_clone.send_signal(15);
                    thread::sleep(Duration::from_millis(500));
                }
                // Force kill the process
                let _res = child_arc_clone.kill();
            });

            let process_result = child_arc.wait();
            let mut running_handle = running.lock().unwrap();
            *running_handle = true;
            (process_result, timed_out)
        };

        // Very important when using pipes: This parent process is still
        // holding its copies of the write ends, and we have to close them
        // before we read, otherwise the read end will never report EOF.
        // The block above drops everything unnecessary



        let res = match process_result {
            Ok(exit_status) => {
                let mut stdout = vec![];
                let mut stderr = vec![];
                stdout_reader
                    .read_to_end(&mut stdout)
                    .map_err(TmcError::ReadStdio)?;
                stderr_reader
                    .read_to_end(&mut stderr)
                    .map_err(TmcError::ReadStdio)?;

                Output { status: exit_status, stdout: stdout, stderr: stderr}
            }
            Err(e) => {
                if let std::io::ErrorKind::NotFound = e.kind() {
                    return Err(TmcError::Command(CommandError::NotFound {
                        name: name,
                        path: path,
                        source: e,
                    }));
                } else {
                    return Err(TmcError::Command(CommandError::FailedToRun(self_string2, e)));
                }
            }
        };

        if timed_out.lock().unwrap().clone() {
            return Ok(OutputWithTimeout::Timeout { stdout: res.stdout, stderr: res.stderr});
        }

        return Ok(OutputWithTimeout::Output(res));
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
#[derive(Debug)]
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
            panic!(format!("Unexpected result {:?}", res));
        }
    }
}
