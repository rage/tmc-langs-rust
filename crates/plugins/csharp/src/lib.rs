#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! TMC language plugin for C#.

mod cs_test_result;
mod error;
mod plugin;
mod policy;

pub use self::{error::CSharpError, plugin::CSharpPlugin, policy::CSharpStudentFilePolicy};
