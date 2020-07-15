//! Contains functionality for dealing with projects.

pub mod domain;
pub mod error;
pub mod io;
pub mod plugin;
pub mod policy;

pub use error::TmcError;
pub use plugin::LanguagePlugin;
pub use policy::StudentFilePolicy;

use domain::TmcProjectYml;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, TmcError>;

pub struct CommandWithTimeout<'a>(pub &'a mut Command);

impl CommandWithTimeout<'_> {
    pub fn wait_with_timeout(
        &mut self,
        name: &'static str,
        timeout: Option<Duration>,
    ) -> Result<Output> {
        match timeout {
            Some(timeout) => {
                // spawn process and init timer
                let mut child = self
                    .0
                    .spawn()
                    .map_err(|e| TmcError::CommandSpawn(name, e))?;
                let timer = Instant::now();
                loop {
                    match child.try_wait().map_err(|e| TmcError::Process(e))? {
                        Some(_exit_status) => {
                            // done, get output
                            return child
                                .wait_with_output()
                                .map_err(|e| TmcError::CommandFailed(name, e));
                        }
                        None => {
                            // still running, check timeout
                            if timer.elapsed() > timeout {
                                child.kill().map_err(|e| TmcError::Process(e))?;
                                return Err(TmcError::TestTimeout(timer.elapsed()));
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
        let res = out.wait_with_timeout("sleep", Some(Duration::from_millis(100)));
        if let Err(TmcError::TestTimeout(_)) = res {
        } else {
            panic!("unexpected result: {:?}", res);
        }
    }
}
