//! Contains the Credentials struct for authenticating with tmc-server.

use crate::{LangsError, tmc::Token};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tmc_langs_util::{
    deserialize,
    file_util::{self, Lock, LockOptions},
};

/// Credentials for authenticating with tmc-server.
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
        log::debug!("Loading credentials from {}", credentials_path.display());

        let mut credentials_lock = Lock::file(&credentials_path, LockOptions::Read)?;
        let credentials_guard = credentials_lock.lock()?;
        match deserialize::json_from_reader(credentials_guard.get_file()) {
            Ok(token) => Ok(Some(Credentials {
                path: credentials_path,
                token,
            })),
            Err(e) => {
                log::error!("Failed to deserialize credentials.json due to \"{e}\", deleting");
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
        let mut credentials_lock = Lock::file(&credentials_path, LockOptions::WriteTruncate)?;
        let mut credentials_guard = credentials_lock.lock()?;
        // write token
        if let Err(e) = serde_json::to_writer(credentials_guard.get_file_mut(), &token) {
            // failed to write token, removing credentials file
            file_util::remove_file(&credentials_path)?;
            return Err(LangsError::Json(e));
        }
        Ok(())
    }

    pub fn remove(self) -> Result<(), LangsError> {
        file_util::remove_file_locked(self.path)?;
        Ok(())
    }

    pub fn token(&self) -> Token {
        self.token.clone()
    }
}
