//! Contains various error types.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid token. Deleted credentials file")]
pub struct InvalidTokenError {
    pub source: anyhow::Error,
}

/// tmc.mooc.fi refused the courses.mooc.fi access token with a 401.
#[derive(Debug, Error)]
#[error("tmc.mooc.fi did not accept the courses.mooc.fi login")]
pub struct TmcLoginNotAcceptedError {
    pub source: anyhow::Error,
}

#[derive(Debug, Error)]
#[error("Error running tests on sandbox")]
pub struct SandboxTestError {
    pub path: Option<PathBuf>,
    pub source: anyhow::Error,
}
