//! Contains various error types.

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs::TmcExerciseDownload;

#[derive(Debug, Error)]
#[error("Invalid token. Deleted credentials file")]
pub struct InvalidTokenError {
    pub source: anyhow::Error,
}

#[derive(Debug, Error)]
#[error("Error running tests on sandbox")]
pub struct SandboxTestError {
    pub path: Option<PathBuf>,
    pub source: anyhow::Error,
}

#[derive(Debug, Error)]
#[error("Failed to download one or more exercises")]
pub struct DownloadsFailedError {
    pub downloaded: Vec<TmcExerciseDownload>,
    pub skipped: Vec<TmcExerciseDownload>,
    pub failed: Vec<(TmcExerciseDownload, Vec<String>)>,
}
