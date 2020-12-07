//! Contains functionality for dealing with projects.

pub mod command;
pub mod domain;
pub mod error;
pub mod file_util;
pub mod meta_syntax;
pub mod plugin;
pub mod policy;
pub mod tmc_project_yml;

pub use self::error::TmcError;
pub use self::plugin::LanguagePlugin;
pub use self::policy::StudentFilePolicy;
pub use self::tmc_project_yml::TmcProjectYml;
pub use anyhow;
pub use nom;
pub use subprocess;
pub use zip;
