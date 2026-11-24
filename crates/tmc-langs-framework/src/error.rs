//! Contains the TmcError type.

pub use nom_language::error::VerboseError;
use std::{io, path::PathBuf, time::Duration};
pub use subprocess::ExitStatus;
use thiserror::Error;
pub use tmc_langs_util::{FileError, YamlError};
pub use zip::result::ZipError;

// todo: make util error type and move variants there
#[derive(Error, Debug)]
pub enum TmcError {
    #[error("Failed to seek archive")]
    Seek(#[source] std::io::Error),

    #[error("Failed to hash file")]
    Hash(#[source] std::io::Error),

    #[error("Failed to read zip file")]
    ZipRead(#[source] std::io::Error),
    #[error("Failed to read file inside zip archive with path {0}")]
    ZipReadFile(PathBuf, #[source] std::io::Error),
    #[error("Failed to write file at {0} to zip archive")]
    ZipWrite(PathBuf, #[source] std::io::Error),
    #[error("Unsafe archive entry {entry}: it {reason}")]
    UnsafeArchiveEntry { entry: String, reason: &'static str },
    #[error("Archive entry {entry} exceeds the extraction limit of {max} {limit}")]
    ArchiveExtractionLimitExceeded {
        entry: String,
        limit: &'static str,
        max: u64,
    },
    #[error("Path {0} cannot be used as an archive entry name")]
    InvalidArchiveEntryPath(PathBuf),
    #[error("Failed to read tar archive")]
    TarRead(#[source] std::io::Error),
    #[error("Failed to write tar archive")]
    TarWrite(#[source] std::io::Error),
    #[error("Failed to read zstd archive")]
    ZstdRead(#[source] std::io::Error),
    #[error("Failed to write zstd archive")]
    ZstdWrite(#[source] std::io::Error),
    #[error("Archive size limit exceeded when compressing project (limit: {limit} MB)")]
    ArchiveSizeLimitExceeded { limit: u32 },

    #[error("Failed to read line")]
    ReadLine(#[source] std::io::Error),
    #[error("Failed to get file metadata")]
    FileMetadata(#[source] std::io::Error),
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("File {0} not in given project root {1}")]
    FileNotInProject(PathBuf, PathBuf),
    #[error("Error while parsing available points from {0}")]
    PointParse(PathBuf, #[source] VerboseError<String>),

    #[error("No project directory found in archive")]
    NoProjectDirInArchive,
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
    Popen(String, #[source] io::Error),
    #[error("Could not find `{tool}` (command: {cmd}). {hint}")]
    NotFound {
        cmd: String,
        /// File name of the program that was not found.
        tool: String,
        /// What to install or restart; see [`crate::missing_tool_hint`].
        hint: String,
        source: io::Error,
    },
    #[error(
        "Could not start {command}: the file was blocked, denied or removed right after it was \
         built, which antivirus software often does to freshly built programs. Add the exercise \
         folder to your antivirus exclusions (or temporarily allow the file) and run the tests again."
    )]
    ExecutableBlocked { command: String, source: io::Error },
    #[error("Failed to run command {0}")]
    FailedToRun(String, #[source] io::Error),
    #[error(
        "Command {command} exited with status {status:?}{}",
        describe_output(stdout, stderr)
    )]
    Failed {
        command: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error(
        "Command {command} timed out after {} seconds.{}",
        .timeout.as_secs(),
        describe_output(stdout, stderr)
    )]
    TimeOut {
        command: String,
        timeout: Duration,
        stdout: String,
        stderr: String,
    },
    #[error("Failed to terminate command {0}")]
    Terminate(String, #[source] std::io::Error),
}

/// Most characters of each output stream [`describe_output`] keeps, from the end, where a
/// failing program says what went wrong.
const MAX_OUTPUT_CHARS: usize = 4000;

/// A command's output for an error message: each non-empty stream on its own lines, its tail
/// when long, or nothing when both are empty. Plugins' error messages are the only place a
/// client sees why a tool failed.
pub fn describe_output(stdout: &str, stderr: &str) -> String {
    let mut description = String::new();
    for (name, output) in [("stdout", stdout), ("stderr", stderr)] {
        let output = output.trim();
        if output.is_empty() {
            continue;
        }
        let char_count = output.chars().count();
        description.push_str(&format!("\n{name}:\n"));
        if char_count > MAX_OUTPUT_CHARS {
            description.push('…');
            description.extend(output.chars().skip(char_count - MAX_OUTPUT_CHARS));
        } else {
            description.push_str(output);
        }
    }
    description
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn describe_output_leaves_out_empty_streams() {
        assert_eq!(describe_output(" \n", ""), "");
        assert_eq!(
            describe_output("", "Error occurred during initialization of VM\n"),
            "\nstderr:\nError occurred during initialization of VM"
        );
    }

    #[test]
    fn describe_output_keeps_the_tail_of_a_long_stream() {
        let stdout = format!("{}the cause", "x".repeat(MAX_OUTPUT_CHARS * 2));
        let description = describe_output(&stdout, "");
        assert!(description.ends_with("the cause"));
        assert_eq!(
            description.chars().count(),
            "\nstdout:\n…".chars().count() + MAX_OUTPUT_CHARS
        );
    }
}
