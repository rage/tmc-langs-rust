//! Contains functionality for dealing with projects.

pub mod command;
pub mod domain;
pub mod error;
pub mod io;
pub mod meta_syntax;
pub mod plugin;
pub mod policy;

pub use self::error::TmcError;
pub use self::policy::StudentFilePolicy;
pub use anyhow;
pub use nom;
pub use plugin::LanguagePlugin;
pub use subprocess;
pub use zip;

use self::domain::TmcProjectYml;
