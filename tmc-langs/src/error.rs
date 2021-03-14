use std::path::PathBuf;

use thiserror::Error;

use crate::course_refresher::ModeBits;
#[derive(Error, Debug)]
pub enum LangsError {
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to create temporary directory")]
    TempDir(#[source] std::io::Error),
    #[error("Invalid parameter key/value: {0}")]
    InvalidParam(String, #[source] ParamError),
    #[error("Error compressing file at {0} with zstd")]
    Zstd(PathBuf, #[source] std::io::Error),
    #[error("Error retrieving file handle from tar builder")]
    TarIntoInner(#[source] std::io::Error),
    #[error("Error finishing tar")]
    TarFinish(#[source] std::io::Error),
    #[error("Error appending path {0} to tar")]
    TarAppend(PathBuf, #[source] std::io::Error),
    #[error("Failed to aquire mutex")]
    MutexError,
    #[error("No project directory found in archive during unzip")]
    NoProjectDirInZip(PathBuf),
    #[error("Error while writing file to zip")]
    ZipWrite(#[source] std::io::Error),
    #[error("Failed to parse file {0}")]
    SubmissionParse(PathBuf, #[source] Box<Self>),

    #[error("Cache path {0} was invalid. Not a valid UTF-8 string or did not contain a cache version after a dash")]
    InvalidCachePath(PathBuf),
    #[error("Path {0} contained a dash '-' which is currently not allowed")]
    InvalidDirectory(PathBuf),

    #[cfg(unix)]
    #[error("Error changing permissions of {0}")]
    NixPermissionChange(PathBuf, #[source] nix::Error),
    #[cfg(unix)]
    #[error("Invalid chmod flag: {0}")]
    NixFlag(ModeBits),

    #[error(transparent)]
    Tmc(#[from] tmc_langs_framework::TmcError),
    #[error(transparent)]
    Plugin(#[from] tmc_langs_plugins::PluginError),
    #[error(transparent)]
    FileError(#[from] tmc_langs_util::FileError),
    #[error(transparent)]
    Heim(#[from] heim::Error),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Error)]
pub enum ParamError {
    #[error("Parameter key/value was empty")]
    Empty,
    #[error("Invalid character found in key/value: {0}")]
    InvalidChar(char),
}
