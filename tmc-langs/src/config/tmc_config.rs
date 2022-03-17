//! Handles the CLI's configuration file.

use crate::error::LangsError;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    env,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tmc_langs_util::{deserialize, file_util, FileError};
use toml::{value::Table, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// The main configuration file. A separate one is used for each client.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct TmcConfig {
    #[serde(alias = "projects-dir")]
    pub projects_dir: PathBuf,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub table: Table,
}

impl TmcConfig {
    pub fn get(&self, key: &str) -> ConfigValue {
        match key {
            "projects-dir" => ConfigValue::Path(Cow::Borrowed(&self.projects_dir)),
            _ => ConfigValue::Value(self.table.get(key).map(Cow::Borrowed)),
        }
    }

    pub fn insert(&mut self, key: String, value: Value) -> Result<(), LangsError> {
        match key.as_str() {
            "projects-dir" => {
                if let Value::String(value) = value {
                    let path = PathBuf::from(value);
                    self.set_projects_dir(path)?;
                } else {
                    return Err(LangsError::ProjectsDirNotString);
                }
            }
            _ => {
                self.table.insert(key, value);
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<Option<Value>, LangsError> {
        match key {
            "projects-dir" => Err(LangsError::NoProjectsDir),
            _ => Ok(self.table.remove(key)),
        }
    }

    pub fn set_projects_dir(&mut self, mut target: PathBuf) -> Result<PathBuf, LangsError> {
        // check if the directory is empty or not
        if file_util::read_dir(&target)?.next().is_some() {
            return Err(LangsError::NonEmptyDir(target));
        }
        std::mem::swap(&mut self.projects_dir, &mut target);
        Ok(target)
    }

    pub fn save(self, path: &Path) -> Result<(), LangsError> {
        if let Some(parent) = path.parent() {
            file_util::create_dir_all(parent)?;
        }
        let mut lock = file_util::create_file_lock(&path)?;
        let mut guard = lock
            .write()
            .map_err(|e| FileError::FdLock(path.to_path_buf(), e))?;

        let toml = toml::to_string_pretty(&self)?;
        guard
            .write_all(toml.as_bytes())
            .map_err(|e| FileError::FileWrite(path.to_path_buf(), e))?;
        Ok(())
    }

    pub fn reset(client_name: &str) -> Result<(), LangsError> {
        let path = Self::get_location(client_name)?;
        Self::init_at(client_name, &path)?; // init locks the file
        Ok(())
    }

    pub fn load(client_name: &str, path: &Path) -> Result<TmcConfig, LangsError> {
        // try to open config file
        let config = match file_util::open_file_lock(path) {
            Ok(mut lock) => {
                // found config file, lock and read
                let mut guard = lock
                    .write()
                    .map_err(|e| FileError::FdLock(path.to_path_buf(), e))?;
                let mut buf = String::new();
                let _bytes = guard
                    .read_to_string(&mut buf)
                    .map_err(|e| FileError::FileRead(path.to_path_buf(), e))?;
                match deserialize::toml_from_str(&buf) {
                    // successfully read file, try to deserialize
                    Ok(config) => config, // successfully read and deserialized the config
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize config at {} due to {}, resetting",
                            path.display(),
                            e
                        );
                        drop(guard); // unlock file before recreating it
                        Self::init_at(client_name, path)?
                    }
                }
            }
            Err(e) => {
                // failed to open config file, create new one
                log::info!(
                    "could not open config file at {} due to {}, initializing a new config file",
                    path.display(),
                    e
                );
                // todo: check the cause to make sure this makes sense, might be necessary to propagate some error kinds
                Self::init_at(client_name, path)?
            }
        };

        if !config.projects_dir.exists() {
            file_util::create_dir_all(&config.projects_dir)?;
        }
        Ok(config)
    }

    fn get_default_projects_dir() -> Result<PathBuf, LangsError> {
        let data_dir = match env::var("TMC_LANGS_DEFAULT_PROJECTS_DIR") {
            Ok(v) => PathBuf::from(v),
            Err(_) => dirs::data_local_dir()
                .ok_or(LangsError::NoLocalDataDir)?
                .join("tmc"),
        };
        Ok(data_dir)
    }

    // initializes the default configuration file at the given path
    fn init_at(client_name: &str, path: &Path) -> Result<TmcConfig, LangsError> {
        if let Some(parent) = path.parent() {
            file_util::create_dir_all(parent)?;
        }

        let mut lock = file_util::create_file_lock(path)?;
        let mut guard = lock
            .write()
            .map_err(|e| FileError::FdLock(path.to_path_buf(), e))?;

        let default_project_dir =
            Self::get_default_projects_dir()?.join(Self::get_client_stub(client_name));
        file_util::create_dir_all(&default_project_dir)?;

        let config = TmcConfig {
            projects_dir: default_project_dir,
            table: Table::new(),
        };

        let toml = toml::to_string_pretty(&config).expect("this should never fail");
        guard
            .write_all(toml.as_bytes())
            .map_err(|e| FileError::FileWrite(path.to_path_buf(), e))?;
        Ok(config)
    }

    // path to the configuration file
    pub fn get_location(client_name: &str) -> Result<PathBuf, LangsError> {
        super::get_tmc_dir(client_name).map(|dir| dir.join("config.toml"))
    }

    // some clients use a different name for the directory
    fn get_client_stub(client: &str) -> &str {
        match client {
            "vscode_plugin" => "vscode",
            s => s,
        }
    }
}

/// A setting in a TmcConfig file.
#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
pub enum ConfigValue<'a> {
    Value(Option<Cow<'a, Value>>),
    Path(Cow<'a, Path>),
}

impl ConfigValue<'_> {
    pub fn into_owned(self) -> ConfigValue<'static> {
        match self {
            Self::Value(Some(v)) => ConfigValue::Value(Some(Cow::Owned(v.into_owned()))),
            Self::Value(None) => ConfigValue::Value(None),
            Self::Path(p) => ConfigValue::Path(Cow::Owned(p.into_owned())),
        }
    }
}
