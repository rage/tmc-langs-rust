//! Handles the CLI's configuration files and credentials.

use anyhow::{Context, Error};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use toml::{value::Table, Value};

// base directory for a given plugin's settings files
fn get_tmc_dir(client_name: &str) -> Result<PathBuf, Error> {
    let config_dir = match env::var("TMC_LANGS_CONFIG_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::config_dir().context("Failed to find config directory")?,
    };
    Ok(config_dir.join(format!("tmc-{}", client_name)))
}

// path to the credentials file
pub fn get_credentials_path(client_name: &str) -> Result<PathBuf, Error> {
    get_tmc_dir(client_name).map(|dir| dir.join("credentials.json"))
}

// path to the configuration file
fn get_config_path(client_name: &str) -> Result<PathBuf, Error> {
    get_tmc_dir(client_name).map(|dir| dir.join("config.toml"))
}

// some client use a different name for the directory
fn get_client_stub(client: &str) -> &str {
    match client {
        "vscode_plugin" => "vscode",
        s => s,
    }
}

// initializes the default configuration file at the given path
fn init_config_at(client_name: &str, path: &Path) -> Result<Table, Error> {
    let mut file = File::create(&path)
        .with_context(|| format!("Failed to create new config file at {}", path.display()))?;

    let default_project_dir = dirs::data_local_dir()
        .context("Failed to find local data directory")?
        .join("tmc")
        .join(get_client_stub(client_name));
    fs::create_dir_all(&default_project_dir).with_context(|| {
        format!(
            "Failed to create the TMC default project directory in {}",
            default_project_dir.display()
        )
    })?;

    let mut config = Table::new();
    config.insert(
        "projects-folder".to_string(),
        Value::String(default_project_dir.to_string_lossy().into_owned()),
    );

    let toml = toml::to_string_pretty(&config).context("Failed to serialize config")?;
    file.write_all(toml.as_bytes())
        .with_context(|| format!("Failed to write default config to {}", path.display()))?;
    Ok(config)
}

pub fn load_config(client_name: &str) -> Result<Table, Error> {
    let path = get_config_path(client_name)?;
    match fs::read(&path) {
        Ok(bytes) => match toml::from_slice(&bytes) {
            Ok(config) => Ok(config),
            Err(_) => {
                log::error!(
                    "Failed to deserialize config at {}, deleting",
                    path.display()
                );
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to remove invalid config file at {}", path.display())
                })?;
                init_config_at(client_name, &path)
            }
        },
        Err(_) => init_config_at(client_name, &path),
    }
}

pub fn save_config(client_name: &str, config: Table) -> Result<(), Error> {
    let path = get_config_path(client_name)?;
    let toml = toml::to_string_pretty(&config).context("Failed to serialize HashMap")?;
    fs::write(&path, toml.as_bytes())
        .with_context(|| format!("Failed to write TOML to {}", path.display()))?;
    Ok(())
}

pub fn reset_config(client_name: &str) -> Result<(), Error> {
    let path = get_config_path(client_name)?;
    init_config_at(client_name, &path)?;
    Ok(())
}
