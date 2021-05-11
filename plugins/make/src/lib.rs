#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! TMC plugin for make.

mod check_log;
mod error;
mod plugin;
mod policy;
mod valgrind_log;

pub use error::MakeError;
pub use plugin::MakePlugin;
pub use policy::MakeStudentFilePolicy;
