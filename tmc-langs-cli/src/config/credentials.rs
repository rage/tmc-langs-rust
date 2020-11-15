use crate::Token;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    path: PathBuf,
    token: Token,
}

impl Credentials {
    // path to the credentials file
    fn get_credentials_path(client_name: &str) -> Result<PathBuf> {
        super::get_tmc_dir(client_name).map(|dir| dir.join("credentials.json"))
    }

    /// ### Returns
    /// - Ok(Some) if a credentials file exists and can be deserialized,
    /// - Ok(None) if no credentials file exists, and
    /// - Err if a credentials file exists but cannot be deserialized.
    ///
    /// On Err, the file is deleted.
    pub fn load(client_name: &str) -> Result<Option<Self>> {
        let credentials_path = Self::get_credentials_path(client_name)?;
        if !credentials_path.exists() {
            return Ok(None);
        }

        let file = File::open(&credentials_path).with_context(|| {
            format!(
                "Failed to read credentials file at {}",
                credentials_path.display()
            )
        })?;
        match serde_json::from_reader(file) {
            Ok(token) => Ok(Some(Credentials {
                path: credentials_path,
                token,
            })),
            Err(e) => {
                log::error!(
                    "Failed to deserialize credentials.json due to \"{}\", deleting",
                    e
                );
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove malformed credentials.json file {}",
                        credentials_path.display()
                    )
                })?;
                anyhow::bail!(
                "Failed to deserialize credentials file at {}; removed the file, please try again.",
                credentials_path.display()
            )
            }
        }
    }

    pub fn save(client_name: &str, token: Token) -> Result<()> {
        let credentials_path = Self::get_credentials_path(client_name)?;
        if let Some(p) = credentials_path.parent() {
            fs::create_dir_all(p)
                .with_context(|| format!("Failed to create directory {}", p.display()))?;
        }
        let credentials_file = File::create(&credentials_path)
            .with_context(|| format!("Failed to create file at {}", credentials_path.display()))?;

        // write token
        if let Err(e) = serde_json::to_writer(credentials_file, &token) {
            // failed to write token, removing credentials file
            fs::remove_file(&credentials_path).with_context(|| {
                format!(
                    "Failed to remove empty credentials file after failing to write {}",
                    credentials_path.display()
                )
            })?;
            Err(e).with_context(|| {
                format!(
                    "Failed to write credentials to {}",
                    credentials_path.display()
                )
            })?;
        }
        Ok(())
    }

    pub fn remove(self) -> Result<()> {
        fs::remove_file(&self.path)
            .with_context(|| format!("Failed to remove credentials at {}", self.path.display()))?;
        Ok(())
    }

    pub fn token(&self) -> Token {
        self.token.clone()
    }
}
