pub use nom::error::VerboseError;
use std::{path::PathBuf, time::Duration};
pub use subprocess::{ExitStatus, PopenError};
use thiserror::Error;
pub use tmc_langs_util::{FileError, YamlError};
pub use zip::result::ZipError;

// todo: make util error type and move variants there
#[derive(Error, Debug)]
pub enum TmcError {
    #[error("Failed to read file inside zip archive with path {0}")]
    ZipRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file at {0} to zip archive")]
    ZipWrite(PathBuf, #[source] std::io::Error),
    #[error("Invalid name {0} in zip")]
    ZipName(String),
    #[error("Failed to read tar archive")]
    TarRead(#[source] std::io::Error),
    #[error("Failed to write tar archive")]
    TarWrite(#[source] std::io::Error),
    #[error("Failed to read zstd archive")]
    ZstdRead(#[source] std::io::Error),
    #[error("Failed to write zstd archive")]
    ZstdWrite(#[source] std::io::Error),

    #[error("Failed to read line")]
    ReadLine(#[source] std::io::Error),
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("File {0} not in given project root {1}")]
    FileNotInProject(PathBuf, PathBuf),
    #[error("Error while parsing available points from {0}")]
    PointParse(PathBuf, #[source] VerboseError<String>),

    #[error("No project directory found in archive")]
    NoProjectDirInArchive,
    #[error("Found project dir in zip, but its path contained invalid UTF-8: {0}")]
    ProjectDirInvalidUtf8(PathBuf),
    #[error("Failed to deserialize YAML from file at {0}")]
    YamlDeserialize(PathBuf, #[source] YamlError),

    #[error("Error in plugin")]
    Plugin(#[from] Box<dyn std::error::Error + 'static + Send + Sync>),

    #[error("Failed to run command")]
    Command(#[from] CommandError),
    #[error("File IO error")]
    FileError(#[from] FileError),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    ZipError(#[from] ZipError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
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
