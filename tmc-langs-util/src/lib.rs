pub mod tar;
pub mod task_executor;

use lazy_static::lazy_static;
use thiserror::Error;
use tmc_langs_framework::LanguagePlugin;
use tmc_langs_python3::Python3Plugin;

lazy_static! {
    pub static ref PLUGINS: Vec<Box<dyn LanguagePlugin + Sync>> =
        vec![Box::new(Python3Plugin::new())];
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("No matching plugin found")]
    PluginNotFound,
}

pub type Result<T> = std::result::Result<T, Error>;
