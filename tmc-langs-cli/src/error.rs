use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid token. Deleted credentials file at {path}")]
pub struct InvalidTokenError {
    pub path: PathBuf,
    pub source: anyhow::Error,
}

#[derive(Debug, Error)]
#[error("Error running tests on sandbox")]
pub struct SandboxTestError {
    pub path: Option<PathBuf>,
    pub source: anyhow::Error,
}
