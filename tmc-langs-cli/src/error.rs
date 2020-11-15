use std::path::PathBuf;
use thiserror::Error;

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
