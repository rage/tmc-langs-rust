//! Contains the Credentials struct for authenticating with tmc-server.
//!
//! Read-only: tmc-server tokens are no longer issued (the password grant is
//! gone), so this only loads a token an older version stored, and deletes it once
//! tmc-server rejects it. Reading it must keep working — a user with a valid
//! stored token keeps that session rather than being pushed onto the
//! courses.mooc.fi token mid-session.

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

    pub fn remove(self) -> Result<(), LangsError> {
        file_util::remove_file_locked(self.path)?;
        Ok(())
    }

    pub fn token(&self) -> Token {
        self.token.clone()
    }
}
