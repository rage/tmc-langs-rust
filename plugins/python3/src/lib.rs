#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Implementation of LanguagePlugin for Python 3.

mod error;
mod plugin;
mod policy;
mod python_test_result;

pub use self::error::PythonError;
pub use self::plugin::Python3Plugin;
pub use self::policy::Python3StudentFilePolicy;
