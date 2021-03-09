#[derive(thiserror::Error, Debug)]
pub enum PluginError {
    #[error(transparent)]
    Tmc(#[from] tmc_langs_framework::TmcError),
}
