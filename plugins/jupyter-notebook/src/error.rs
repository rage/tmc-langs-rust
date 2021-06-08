// Jupyter Notebook error type

use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_framework::TmcError;

#[derive(Debug, Error)]
pub enum JupyterNotebookError {
    #[error("Failed to canonicalize path {0}")]
    Canonicalize(PathBuf, #[source] std::io::Error),
    #[error("Error")]
    Tmc(#[from] tmc_langs_framework::TmcError),
}

// conversion from plugin error to TmcError::Plugin
impl From<JupyterNotebookError> for TmcError {
    fn from(err: JupyterNotebookError) -> TmcError {
        TmcError::Plugin(Box::new(err))
    }
}

// conversion from plugin error to a tmc result
impl<T> From<JupyterNotebookError> for Result<T, TmcError> {
    fn from(from: JupyterNotebookError) -> Self {
        Err(TmcError::Plugin(Box::new(from)))
    }
}
