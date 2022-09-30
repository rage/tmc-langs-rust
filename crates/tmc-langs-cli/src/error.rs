use std::path::PathBuf;
use thiserror::Error;
use tmc_langs::ExerciseDownload;

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
    pub downloaded: Vec<ExerciseDownload>,
    pub skipped: Vec<ExerciseDownload>,
    pub failed: Vec<(ExerciseDownload, Vec<String>)>,
}
