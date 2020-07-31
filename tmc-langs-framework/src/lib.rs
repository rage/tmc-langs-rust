//! Contains functionality for dealing with projects.

pub mod domain;
pub mod error;
pub mod io;
pub mod plugin;
pub mod policy;

pub use error::TmcError;
pub use plugin::LanguagePlugin;
pub use policy::StudentFilePolicy;
pub use zip;

use domain::TmcProjectYml;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, TmcError>;

pub struct CommandWithTimeout<'a>(pub &'a mut Command);

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

impl CommandWithTimeout<'_> {
    pub fn wait_with_timeout(
        &mut self,
        name: &'static str,
        timeout: Option<Duration>,
    ) -> Result<OutputWithTimeout> {
        match timeout {
            Some(timeout) => {
                // spawn process and init timer
                let mut child = self
                    .0
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| TmcError::CommandSpawn(name, e))?;
                let timer = Instant::now();

                loop {
                    match child.try_wait().map_err(TmcError::Process)? {
                        Some(_exit_status) => {
                            // done, get output
                            return child
                                .wait_with_output()
                                .map(OutputWithTimeout::Output)
                                .map_err(|e| TmcError::CommandFailed(name, e));
                        }
                        None => {
                            // still running, check timeout
                            if timer.elapsed() > timeout {
                                log::warn!("command {} timed out", name);
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
            // no timeout, block forever
            None => self
                .0
                .output()
                .map(OutputWithTimeout::Output)
                .map_err(|e| TmcError::CommandFailed(name, e)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn timeout() {
        let mut command = Command::new("sleep");
        let mut command = command.arg("1");
        let mut out = CommandWithTimeout(&mut command);
        let res = out
            .wait_with_timeout("sleep", Some(Duration::from_millis(100)))
            .unwrap();
        if let OutputWithTimeout::Timeout { .. } = res {
        } else {
            panic!("unexpected result");
        }
    }
}
