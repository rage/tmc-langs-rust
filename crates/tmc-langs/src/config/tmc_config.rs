//! Handles the CLI's configuration file.

use crate::error::LangsError;
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::Write,
    path::{Path, PathBuf},
};
use tmc_langs_util::{
    FileError, deserialize,
    file_util::{self, Lock, LockOptions},
};
use toml::{Value, value::Table};

/// The main configuration file. A separate one is used for each client.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct TmcConfig {
    // this is not serialized or deserialized, but set while loading
    #[serde(skip)]
    pub location: PathBuf,
    #[serde(alias = "projects-dir")]
    pub projects_dir: PathBuf,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    pub table: Table,
}

impl TmcConfig {
    /// Reads or initialises the config for the given client.
    pub fn load(client_name: &str) -> Result<TmcConfig, LangsError> {
        let path = Self::get_location(client_name)?;
        log::debug!("Loading config at {}", path.display());
        Self::load_from(client_name, path)
    }

    /// Reads or initialises for the client from the given path.
    pub fn load_from(client_name: &str, path: PathBuf) -> Result<TmcConfig, LangsError> {
        // try to open config file
        let config = if path.exists() {
            // found config file
            let data = file_util::read_file_to_string(&path)?;
            match deserialize::toml_from_str::<Self>(&data) {
                // successfully read file, try to deserialize
                Ok(mut config) => {
                    // set the path which was set to default during deserialization
                    config.location = path;
                    config // successfully read and deserialized the config
                }
                Err(e) => {
                    log::error!(
                        "Failed to deserialize config at {} due to {}, resetting",
                        path.display(),
                        e
                    );
                    Self::init_at(client_name, path)?
                }
            }
        } else {
            // failed to open config file, create new one
            log::info!("initializing a new config file at {}", path.display());
            // todo: check the cause to make sure this makes sense, might be necessary to propagate some error kinds
            Self::init_at(client_name, path)?
        };

        if !config.projects_dir.exists() {
            file_util::create_dir_all(&config.projects_dir)?;
        }
        Ok(config)
    }

    // initializes the default configuration file at the given path
    fn init_at(client_name: &str, path: PathBuf) -> Result<TmcConfig, LangsError> {
        if let Some(parent) = path.parent() {
            file_util::create_dir_all(parent)?;
        }

        let mut lock = Lock::file(&path, LockOptions::WriteTruncate)?;
        let mut guard = lock.lock()?;
        let default_project_dir = get_projects_dir_root()?.join(get_client_stub(client_name));
        file_util::create_dir_all(&default_project_dir)?;

        let config = TmcConfig {
            location: path,
            projects_dir: default_project_dir,
            table: Table::new(),
        };

        let toml = toml::to_string_pretty(&config).expect("this should never fail");
        guard
            .get_file_mut()
            .write_all(toml.as_bytes())
            .map_err(|e| FileError::FileWrite(config.location.to_path_buf(), e))?;
        Ok(config)
    }

    /// Returns the projects dir.
    pub fn get_projects_dir(&self) -> &Path {
        &self.projects_dir
    }

    /// Sets the projects dir.
    /// Returns the old projects dir.
    pub fn set_projects_dir(&mut self, mut target: PathBuf) -> Result<PathBuf, LangsError> {
        // check if the directory is empty or not
        if file_util::read_dir(&target)?.next().is_some() {
            return Err(LangsError::NonEmptyDir(target));
        }
        std::mem::swap(&mut self.projects_dir, &mut target);
        Ok(target)
    }

    /// Fetches a value with the given key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.table.get(key)
    }

    /// Inserts a value with the given key and value.
    /// Returns the old value, if any.
    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.table.insert(key, value)
    }

    /// Removes the value with the given key.
    /// Returns the removed value, if any.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.table.remove(key)
    }

    /// Saves the config struct to the given path.
    pub fn save(&mut self) -> Result<(), LangsError> {
        let path = &self.location;
        log::info!("Saving config at {}", path.display());

        log::debug!("Saving config to temporary path");
        let parent = path
            .parent()
            .ok_or_else(|| LangsError::NoParentDir(path.to_path_buf()))?;
        let temp_file = file_util::named_temp_file_in(parent)?;
        let toml = toml::to_string_pretty(&self)?;
        file_util::write_to_file(toml, temp_file.path())?;

        log::debug!("Moving new config over old one");
        temp_file.persist(path)?;
        Ok(())
    }

    /// Reinitialises the config file.
    pub fn reset(client_name: &str) -> Result<(), LangsError> {
        let path = Self::get_location(client_name)?;
        Self::init_at(client_name, path)?; // init locks the file
        Ok(())
    }

    // path to the configuration file
    pub fn get_location(client_name: &str) -> Result<PathBuf, LangsError> {
        super::get_tmc_dir(client_name).map(|dir| dir.join("config.toml"))
    }
}

fn get_projects_dir_root() -> Result<PathBuf, LangsError> {
    let data_dir = match env::var("TMC_LANGS_DEFAULT_PROJECTS_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::data_local_dir()
            .ok_or(LangsError::NoLocalDataDir)?
            .join("tmc"),
    };
    Ok(data_dir)
}

// some clients use a different name for the directory
fn get_client_stub(client: &str) -> &str {
    match client {
        "vscode_plugin" => "vscode",
        s => s,
    }
}
