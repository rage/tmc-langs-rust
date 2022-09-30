#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Language plugin for no_tests exercises.

mod plugin;
mod policy;

pub use plugin::NoTestsPlugin;
pub use policy::NoTestsStudentFilePolicy;
