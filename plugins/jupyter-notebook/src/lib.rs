#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Implementation of LanguagePlugin for Jupyter Notebooks.

mod error;
mod plugin;
mod policy;

pub use self::error::JupyterNotebookError;
pub use self::plugin::JupyterNotebookPlugin;
pub use self::policy::JupyterNotebookStudentPolicy;
