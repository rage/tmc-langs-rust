//! Language plugin for the R language

mod error;
mod plugin;
mod policy;
mod r_run_result;

pub use self::plugin::RPlugin;
pub use self::policy::RStudentFilePolicy;
