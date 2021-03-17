#![deny(clippy::print_stdout, clippy::print_stderr)]

//! TMC language plugin for C#.

mod cs_test_result;
mod error;
mod plugin;
mod policy;

pub use self::error::CSharpError;
pub use self::plugin::CSharpPlugin;
pub use self::policy::CSharpStudentFilePolicy;
