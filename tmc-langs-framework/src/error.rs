use std::path::PathBuf;
use std::time::Duration;
use subprocess::ExitStatus;
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
    #[error("Error while parsing available points: {0}")]
    PointParse(String),

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
    YamlDeserialization(#[from] serde_yaml::Error),
    #[error(transparent)]
    ZipError(#[from] zip::result::ZipError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
}

// == Collection of errors likely to be useful in multiple plugins which can be special cased without needing a plugin's specific error type ==
/// An error caused by a failed attempt to execute an external command.
#[derive(Error, Debug)]
pub enum CommandError {
    #[error("Failed to execute command: {0}")]
    Popen(String, #[source] subprocess::PopenError),
    #[error("The executable for command {cmd} could not be found. Please make sure you have installed it correctly.")]
    NotFound {
        cmd: String,
        source: subprocess::PopenError,
    },
    #[error("Failed to run command {0}")]
    FailedToRun(String, #[source] subprocess::PopenError),
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

/// A wrapper for std::io::Error that provides more context for the failed operations.
#[derive(Error, Debug)]
pub enum FileIo {
    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read file at {0}")]
    FileRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file at {0}")]
    FileWrite(PathBuf, #[source] std::io::Error),
    #[error("Failed to create file at {0}")]
    FileCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove file at {0}")]
    FileRemove(PathBuf, #[source] std::io::Error),
    #[error("Failed to copy file from {from} to {to}")]
    FileCopy {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to move file from {from} to {to}")]
    FileMove {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to clone file handle")]
    FileHandleClone(#[source] std::io::Error),

    #[error("Failed to open directory at {0}")]
    DirOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to read directory at {0}")]
    DirRead(PathBuf, #[source] std::io::Error),
    #[error("Failed to create directory at {0}")]
    DirCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove directory at {0}")]
    DirRemove(PathBuf, #[source] std::io::Error),

    #[error("Failed to rename file {from} to {to}")]
    Rename {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to lock file at path {0}")]
    FdLock(PathBuf, #[source] std::io::Error),

    #[error("Path {0} has no file name")]
    NoFileName(PathBuf),
    #[error("Expected {0} to be a directory, but it was a file")]
    UnexpectedFile(PathBuf),

    #[error("Directory walk error")]
    Walkdir(#[from] walkdir::Error),

    // when there is no meaningful data that can be added to an error
    #[error("transparent")]
    Generic(#[from] std::io::Error),
}
