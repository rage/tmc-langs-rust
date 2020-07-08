//! Contains functionality for dealing with projects.

pub mod domain;
pub mod io;
pub mod plugin;
pub mod policy;

pub use plugin::LanguagePlugin;
pub use policy::StudentFilePolicy;

use domain::TmcProjectYml;
use io::zip;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // IO
    #[error("Failed to open file at {0}: {1}")]
    OpenFile(PathBuf, std::io::Error),
    #[error("Failed to create file at {0}: {1}")]
    CreateFile(PathBuf, std::io::Error),
    #[error("Failed to create dir at {0}: {1}")]
    CreateDir(PathBuf, std::io::Error),
    #[error("Failed to rename {0} to {1}: {2}")]
    Rename(PathBuf, PathBuf, std::io::Error),
    #[error("Failed to write to {0}: {1}")]
    Write(PathBuf, std::io::Error),

    #[error("Path {0} contained invalid UTF8")]
    UTF8(PathBuf),

    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip,
    #[error("Running command '{0}' failed: {1}")]
    CommandFailed(&'static str, std::io::Error),

    #[error("Failed to spawn command: {0}")]
    CommandSpawn(&'static str, std::io::Error),
    #[error("Test timed out")]
    TestTimeout,

    #[error("Error in plugin: {0}")]
    Plugin(Box<dyn std::error::Error + 'static + Send + Sync>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    YamlDeserialization(#[from] serde_yaml::Error),
    #[error(transparent)]
    ZipError(#[from] zip::ZipError),
}

pub type Result<T> = std::result::Result<T, Error>;

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
                let mut child = self.0.spawn().map_err(|e| Error::CommandSpawn(name, e))?;
                let timer = Instant::now();
                loop {
                    match child.try_wait()? {
                        Some(_exit_status) => {
                            // done, get output
                            return child
                                .wait_with_output()
                                .map_err(|e| Error::CommandFailed(name, e));
                        }
                        None => {
                            // still running, check timeout
                            if timer.elapsed() > timeout {
                                child.kill()?;
                                return Err(Error::TestTimeout);
                            }

                            // TODO: gradually increase sleep duration?
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            // no timeout, block forever
            None => self.0.output().map_err(|e| Error::CommandFailed(name, e)),
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
        if let Err(Error::TestTimeout) = res {
        } else {
            panic!("unexpected result: {:?}", res);
        }
    }
}
