use crate::{LangsError, Token};
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::path::PathBuf;
use tmc_langs_util::{file_util, FileError};

#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    path: PathBuf,
    token: Token,
}

impl Credentials {
    // path to the credentials file
    fn get_credentials_path(client_name: &str) -> Result<PathBuf, LangsError> {
        super::get_tmc_dir(client_name).map(|dir| dir.join("credentials.json"))
    }

    /// ### Returns
    /// - Ok(Some) if a credentials file exists and can be deserialized,
    /// - Ok(None) if no credentials file exists, and
    /// - Err if a credentials file exists but cannot be deserialized.
    ///
    /// On Err, the file is deleted.
    pub fn load(client_name: &str) -> Result<Option<Self>, LangsError> {
        let credentials_path = Self::get_credentials_path(client_name)?;
        if !credentials_path.exists() {
            return Ok(None);
        }

        let mut credentials_file = file_util::open_file_lock(&credentials_path)?;
        let guard = credentials_file
            .lock()
            .map_err(|e| FileError::FdLock(credentials_path.clone(), e))?;

        match serde_json::from_reader(guard.deref()) {
            Ok(token) => Ok(Some(Credentials {
                path: credentials_path,
                token,
            })),
            Err(e) => {
                log::error!(
                    "Failed to deserialize credentials.json due to \"{}\", deleting",
                    e
                );
                file_util::remove_file(&credentials_path)?;
                Err(LangsError::DeserializeCredentials(credentials_path, e))
            }
        }
    }

    pub fn save(client_name: &str, token: Token) -> Result<(), LangsError> {
        let credentials_path = Self::get_credentials_path(client_name)?;

        if let Some(p) = credentials_path.parent() {
            file_util::create_dir_all(p)?;
        }
        let mut credentials_file = file_util::create_file_lock(&credentials_path)?;
        let guard = credentials_file
            .lock()
            .map_err(|e| FileError::FdLock(credentials_path.clone(), e))?;

        // write token
        if let Err(e) = serde_json::to_writer(guard.deref(), &token) {
            // failed to write token, removing credentials file
            file_util::remove_file(&credentials_path)?;
            return Err(LangsError::Json(e));
        }
        Ok(())
    }

    pub fn remove(self) -> Result<(), LangsError> {
        file_util::lock!(&self.path);

        file_util::remove_file(&self.path)?;
        Ok(())
    }

    pub fn token(&self) -> Token {
        self.token.clone()
    }
}
