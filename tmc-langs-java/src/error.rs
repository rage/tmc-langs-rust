use thiserror::Error;
use tmc_langs_framework::Error as TmcError;

#[derive(Error, Debug)]
pub enum JavaPluginError {
    #[error("No java.home found in Java properties")]
    NoJavaHome,
    #[error("Maven did not output any class path")]
    NoMvnClassPath,
    #[error("Invalid exercise")]
    InvalidExercise,
    #[error("Failed to run {0}")]
    FailedCommand(&'static str),
}

impl Into<TmcError> for JavaPluginError {
    fn into(self) -> TmcError {
        TmcError::Plugin(Box::new(self))
    }
}

impl<T> Into<Result<T, TmcError>> for JavaPluginError {
    fn into(self) -> Result<T, TmcError> {
        Err(TmcError::Plugin(Box::new(self)))
    }
}
