pub use nom::error::VerboseError;
pub use serde_yaml::Error as YamlError;
pub use subprocess::{ExitStatus, PopenError};
pub use tmc_langs_util::FileIo;
pub use walkdir::Error as WalkDirError;
pub use zip::result::ZipError;

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

// todo: make util error type and move variants there
#[derive(Error, Debug)]
pub enum TmcError {
    // IO
    #[error("File IO error")]
    FileIo(#[from] FileIo),

    #[error("Failed to read file inside zip archive with path {0}")]
    ZipRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file at {0} to zip archive")]
    ZipWrite(PathBuf, #[source] std::io::Error),

    #[error("Error appending to tar")]
    TarAppend(#[source] std::io::Error),
    #[error("Error finishing tar")]
    TarFinish(#[source] std::io::Error),
    #[error("Failed to read line")]
    ReadLine(#[source] std::io::Error),
    #[error("Failed to parse file {0}")]
    SubmissionParse(PathBuf, #[source] Box<Self>),
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("File {0} not in given project root {1}")]
    FileNotInProject(PathBuf, PathBuf),
    #[error("Error while parsing available points from {0}")]
    PointParse(PathBuf, #[source] VerboseError<String>),

    #[error("Path {0} contained no file name")]
    NoFileName(PathBuf),

    #[error("No matching plugin found for {0}")]
    PluginNotFound(PathBuf),
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip,
    #[error("Found project dir in zip, but its path contained invalid UTF-8: {0}")]
    ProjectDirInvalidUtf8(PathBuf),

    #[error("Error in plugin")]
    Plugin(#[from] Box<dyn std::error::Error + 'static + Send + Sync>),

    #[error("Failed to run command")]
    Command(#[from] CommandError),

    #[error(transparent)]
    YamlDeserialization(#[from] YamlError),
    #[error(transparent)]
    ZipError(#[from] ZipError),
    #[error(transparent)]
    WalkDir(#[from] WalkDirError),
}

// == Collection of errors likely to be useful in multiple plugins which can be special cased without needing a plugin's specific error type ==
/// An error caused by a failed attempt to execute an external command.
#[derive(Error, Debug)]
pub enum CommandError {
    #[error("Failed to execute command: {0}")]
    Popen(String, #[source] PopenError),
    #[error("The executable for command {cmd} could not be found. Please make sure you have installed it correctly.")]
    NotFound { cmd: String, source: PopenError },
    #[error("Failed to run command {0}")]
    FailedToRun(String, #[source] PopenError),
    #[error("Command {command} exited with status {status:?}")]
    Failed {
        command: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("Command {command} timed out after {} seconds.", .timeout.as_secs())]
    TimeOut {
        command: String,
        timeout: Duration,
        stdout: String,
        stderr: String,
    },
    #[error("Failed to terminate command {0}")]
    Terminate(String, #[source] std::io::Error),
}
