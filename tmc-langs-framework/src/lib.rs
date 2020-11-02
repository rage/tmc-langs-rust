//! Contains functionality for dealing with projects.

pub mod command;
pub mod domain;
pub mod error;
pub mod io;
pub mod plugin;
pub mod policy;

pub use anyhow;
use domain::TmcProjectYml;
pub use error::TmcError;
pub use nom;
pub use plugin::LanguagePlugin;
pub use policy::StudentFilePolicy;
pub use subprocess;
pub use zip;
