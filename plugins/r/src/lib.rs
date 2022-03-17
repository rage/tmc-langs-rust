#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Language plugin for the R language

mod error;
mod plugin;
mod policy;
mod r_run_result;

pub use self::{plugin::RPlugin, policy::RStudentFilePolicy};
