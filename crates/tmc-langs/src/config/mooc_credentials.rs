//! Credentials for authenticating with the Courses MOOC backend.
//!
//! Stored separately from the legacy tmc `credentials.json` in a
//! `credentials_mooc.json` file in the same config directory. The legacy file
//! holds a tmc-server password-grant token that is worthless to the mooc OAuth2
//! device-flow model, so it is deliberately left untouched and NOT migrated.
//!
//! The token is wrapped with the time it was obtained so its expiry survives a
//! reload: the stored `oauth2` token carries only a relative `expires_in`, and
//! `obtained_at` turns that into an absolute deadline the loader can check.

use crate::error::LangsError;
use chrono::{DateTime, Utc};
use oauth2::TokenResponse;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tmc_langs_util::{
    deserialize,
    file_util::{self, Lock, LockOptions},
};
use tmc_mooc_client::{self as mooc, api};
use url::Url;

/// Refresh proactively once the access token has less than this many seconds of
/// life left, so a token that is about to expire is renewed before it is used.
const EXPIRY_MARGIN_SECS: i64 = 60;

/// Serializes refreshes between threads of this process. The on-disk file lock
/// serializes across processes, but the underlying POSIX lock is process-scoped
/// and does not serialize threads within one process; this mutex closes that gap
/// so N concurrent in-process loaders still collapse to a single refresh.
static REFRESH_LOCK: Mutex<()> = Mutex::new(());

/// The on-disk wrapper: the oauth2 token plus when it was obtained.
#[derive(Debug, Serialize, Deserialize)]
struct StoredCredentials {
    token: api::Token,
    obtained_at: DateTime<Utc>,
}

impl StoredCredentials {
    /// The absolute time the access token expires, if its lifetime is known.
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        let expires_in = self.token.expires_in()?;
        let expires_in = chrono::Duration::from_std(expires_in).ok()?;
        Some(self.obtained_at + expires_in)
    }

    /// Whether the access token is still valid with at least `margin_secs` of
    /// life left. A token with no known lifetime is treated as valid (the
    /// refresh-on-401 path is the backstop if the server disagrees).
    fn is_valid_with_margin(&self, margin_secs: i64) -> bool {
        match self.expires_at() {
            Some(expires_at) => Utc::now() + chrono::Duration::seconds(margin_secs) < expires_at,
            None => true,
        }
    }

    fn refresh_token_secret(&self) -> Option<String> {
        self.token.refresh_token().map(|t| t.secret().clone())
    }
}

/// Loaded mooc credentials, tied to the file they came from so they can be
/// removed when a token is rejected.
#[derive(Debug)]
pub struct MoocCredentials {
    path: PathBuf,
    stored: StoredCredentials,
}

impl MoocCredentials {
    fn get_credentials_path(client_name: &str) -> Result<PathBuf, LangsError> {
        super::get_tmc_dir(client_name).map(|dir| dir.join("credentials_mooc.json"))
    }

    /// The stored token, for setting on a client or reporting login status.
    pub fn token(&self) -> api::Token {
        self.stored.token.clone()
    }

    /// The stored access token's secret, e.g. to identify the token that a 401
    /// rejected without exposing the `oauth2` types to callers.
    pub fn access_token(&self) -> String {
        self.stored.token.access_token().secret().clone()
    }

    /// Deletes the credentials file.
    pub fn remove(self) -> Result<(), LangsError> {
        file_util::remove_file_locked(self.path)?;
        Ok(())
    }

    /// Persists a freshly obtained token, stamping it with the current time.
    ///
    /// Writes under the exclusive file lock (mirroring [`Self::refresh_locked`])
    /// so a concurrent locked reader can never observe a torn/empty file, and the
    /// write itself is atomic (temp file + rename, see [`write_stored`]).
    pub fn save(client_name: &str, token: api::Token) -> Result<(), LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        let stored = StoredCredentials {
            token,
            obtained_at: Utc::now(),
        };
        // Exclusive file lock; created if missing. Serializes this write against
        // a concurrent refresh or a second `save`.
        let mut lock = Lock::file(&path, LockOptions::WriteCreate)?;
        let _guard = lock.lock()?;
        write_stored(&path, &stored)
    }

    /// Loads the credentials without refreshing.
    ///
    /// Returns `Ok(None)` if no file exists. On a corrupt file the file is
    /// deleted and an error returned, mirroring the tmc `Credentials::load`.
    pub fn load(client_name: &str) -> Result<Option<Self>, LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        if !path.exists() {
            return Ok(None);
        }
        log::debug!("Loading mooc credentials from {}", path.display());
        match read_stored_shared(&path)? {
            Ok(stored) => Ok(Some(Self { path, stored })),
            Err(e) => {
                log::error!("Failed to deserialize {}: {e}, deleting", path.display());
                file_util::remove_file(&path)?;
                Err(LangsError::DeserializeCredentials(path, e))
            }
        }
    }

    /// Loads the credentials, refreshing the token first if it is expired (or
    /// within [`EXPIRY_MARGIN_SECS`] of expiring).
    ///
    /// The common case — a still-valid token — takes a shared read lock only.
    /// When a refresh is needed, an exclusive lock is taken and the file is
    /// re-read under it (double-checked): a concurrent process may have already
    /// refreshed, in which case its result is used and no second refresh is
    /// issued. This collapses N racing refreshers to a single network refresh.
    ///
    /// A *permanent* refresh failure — the refresh token was rejected as
    /// invalid/revoked/expired — deletes the credentials and returns `Ok(None)`,
    /// so the caller proceeds unauthenticated and hits the usual "not logged in"
    /// path. A *transient* failure (connection error, timeout, 5xx, unparseable
    /// response) leaves the file untouched and returns `Err`, so the command
    /// fails with a connection-error kind and the user can retry without being
    /// logged out.
    pub fn load_valid(
        client_name: &str,
        root_url: &Url,
        client_id: &str,
    ) -> Result<Option<Self>, LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        if !path.exists() {
            return Ok(None);
        }

        // Hot path: shared lock, read, use directly if still valid.
        match read_stored_shared(&path)? {
            Ok(stored) => {
                if stored.is_valid_with_margin(EXPIRY_MARGIN_SECS) {
                    return Ok(Some(Self { path, stored }));
                }
            }
            Err(e) => {
                log::error!("Failed to deserialize {}: {e}, deleting", path.display());
                file_util::remove_file(&path)?;
                return Err(LangsError::DeserializeCredentials(path, e));
            }
        }

        // Cold path: expired token, refresh under an exclusive lock.
        Self::refresh_locked(&path, root_url, client_id, None)
    }

    /// After an API call was rejected with 401, refreshes the token once under
    /// an exclusive lock and returns the renewed credentials, so the caller can
    /// retry the call. If the stored token has already been rotated away from
    /// the rejected one (another process refreshed), that newer token is
    /// returned without a fresh refresh. Returns `Ok(None)` — leaving the file
    /// deleted — when there is nothing to refresh with or the refresh token was
    /// permanently rejected. A transient refresh failure instead returns `Err`
    /// with the file left intact, so the caller surfaces a connection-error kind
    /// and the user keeps their session to retry.
    pub fn refresh_after_401(
        client_name: &str,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: &str,
    ) -> Result<Option<Self>, LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        if !path.exists() {
            return Ok(None);
        }
        Self::refresh_locked(&path, root_url, client_id, Some(rejected_access_token))
    }

    /// Exclusive-lock refresh shared by the proactive and on-401 paths. Re-reads
    /// the file under the lock (double-checked) before deciding to refresh.
    fn refresh_locked(
        path: &Path,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<Option<Self>, LangsError> {
        // Serialize in-process threads first (the file lock below only serializes
        // across processes).
        let _in_process = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Exclusive file lock; created if missing so the lock still serializes
        // even if a racing process deleted the file after a failed refresh.
        let mut lock = Lock::file(path, LockOptions::WriteCreate)?;
        let _guard = lock.lock()?;

        // Double-checked re-read under the lock (a separate handle is fine: we
        // hold the advisory lock and other processes are blocked on it).
        if !path.exists() {
            return Ok(None);
        }
        let stored = match read_stored(path) {
            Ok(stored) => stored,
            Err(e) => {
                log::error!("Failed to deserialize {}: {e}, deleting", path.display());
                file_util::remove_file(path)?;
                return Ok(None);
            }
        };

        match rejected_access_token {
            // On-401: if the stored token differs from the rejected one, a
            // concurrent refresh already rotated it; use it without refreshing.
            Some(rejected) => {
                if stored.token.access_token().secret() != rejected {
                    return Ok(Some(Self {
                        path: path.to_path_buf(),
                        stored,
                    }));
                }
            }
            // Proactive: another process may have refreshed while we waited for
            // the lock; if so, the re-read token is now valid — use it.
            None => {
                if stored.is_valid_with_margin(EXPIRY_MARGIN_SECS) {
                    return Ok(Some(Self {
                        path: path.to_path_buf(),
                        stored,
                    }));
                }
            }
        }

        let Some(refresh_secret) = stored.refresh_token_secret() else {
            log::warn!("mooc credentials have no refresh token; deleting");
            file_util::remove_file(path)?;
            return Ok(None);
        };

        match mooc::refresh_token(root_url, client_id, &refresh_secret) {
            Ok(token) => {
                let stored = StoredCredentials {
                    token,
                    obtained_at: Utc::now(),
                };
                write_stored(path, &stored)?;
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    stored,
                }))
            }
            // Permanent rejection: the refresh token is no longer valid (revoked
            // or expired). Delete the credentials and report "not logged in" so
            // the user is sent through the login flow again.
            Err(e) if matches!(*e, mooc::MoocClientError::RefreshTokenRejected { .. }) => {
                log::warn!("mooc refresh token rejected ({e}); deleting credentials");
                file_util::remove_file(path)?;
                Ok(None)
            }
            // Transient failure (connection error, timeout, 5xx, unparseable
            // response). Keep the credentials untouched so a later retry can
            // succeed, and propagate the error so the command fails with a
            // connection-error kind rather than logging the user out.
            Err(e) => {
                log::warn!("mooc token refresh failed transiently ({e}); keeping credentials");
                Err(LangsError::MoocClient(e))
            }
        }
    }

    /// Runs `op` once; on an auth rejection (401 /
    /// [`mooc::MoocClientError::NotAuthenticated`]), refreshes the stored
    /// credentials and retries `op` exactly once against the refreshed token.
    ///
    /// Applied at the point of each network call rather than around a whole
    /// multi-call subcommand, so a 401 on request N never causes requests
    /// `1..N-1` to be repeated -- wrong for a non-idempotent request like
    /// submitting an exercise. See [`MoocAuthFailure`] for the failure modes.
    pub fn call_with_refresh<T>(
        client_name: &str,
        root_url: &Url,
        client_id: &str,
        client: &mooc::MoocClient,
        mut op: impl FnMut(&mooc::MoocClient) -> mooc::MoocClientResult<T>,
    ) -> Result<T, MoocAuthFailure> {
        let err = match op(client) {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        if !is_auth_rejection(&err) {
            return Err(MoocAuthFailure::Other(err));
        }

        // Nothing to refresh with (no token was ever set on this client) --
        // there is no path to recovery.
        let Some(rejected_access_token) = client.access_token() else {
            return Err(MoocAuthFailure::Permanent(err));
        };

        // Refresh, retrying once more if the refresh call itself fails
        // transiently (a network blip talking to the token endpoint, not a
        // rejection of the refresh token itself).
        let mut refresh_result =
            Self::refresh_after_401(client_name, root_url, client_id, &rejected_access_token);
        if refresh_result.is_err() {
            refresh_result =
                Self::refresh_after_401(client_name, root_url, client_id, &rejected_access_token);
        }

        match refresh_result {
            Ok(Some(refreshed)) => {
                let mut refreshed_client = client.clone();
                refreshed_client.set_token(refreshed.token());
                match op(&refreshed_client) {
                    Ok(value) => Ok(value),
                    Err(retry_err) if is_auth_rejection(&retry_err) => {
                        log::error!(
                            "mooc call was still rejected after a token refresh, deleting credentials"
                        );
                        let _ = refreshed.remove();
                        Err(MoocAuthFailure::Permanent(retry_err))
                    }
                    Err(retry_err) => Err(MoocAuthFailure::Other(retry_err)),
                }
            }
            // Refresh permanently failed (the refresh token was rejected) or
            // there was nothing to refresh with; `refresh_after_401` already
            // deleted the credentials in that case.
            Ok(None) => Err(MoocAuthFailure::Permanent(err)),
            // The refresh call itself failed transiently, twice in a row.
            // Credentials are untouched; surface the original rejection as a
            // non-permanent failure so a later call can retry.
            Err(_transient) => Err(MoocAuthFailure::Other(err)),
        }
    }
}

/// Whether a mooc client error indicates the token was rejected (401 or the
/// dedicated `NotAuthenticated` variant).
fn is_auth_rejection(err: &mooc::MoocClientError) -> bool {
    matches!(err, mooc::MoocClientError::NotAuthenticated)
        || matches!(err, mooc::MoocClientError::HttpError { status, .. } if status.as_u16() == 401)
}

/// Bundles the parameters [`MoocCredentials::call_with_refresh`] needs, so
/// functions that make several mooc API calls can thread one value through
/// instead of three loose ones.
#[derive(Debug, Clone)]
pub struct MoocAuth {
    client_name: String,
    root_url: Url,
    client_id: String,
}

impl MoocAuth {
    pub fn new(
        client_name: impl Into<String>,
        root_url: Url,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            root_url,
            client_id: client_id.into(),
        }
    }

    /// Runs `op` once against `client`, refreshing and retrying once on an
    /// auth rejection. See [`MoocCredentials::call_with_refresh`] for the full
    /// contract.
    pub fn call<T>(
        &self,
        client: &mooc::MoocClient,
        op: impl FnMut(&mooc::MoocClient) -> mooc::MoocClientResult<T>,
    ) -> Result<T, MoocAuthFailure> {
        MoocCredentials::call_with_refresh(
            &self.client_name,
            &self.root_url,
            &self.client_id,
            client,
            op,
        )
    }
}

/// The outcome of a failed [`MoocAuth::call`] / [`MoocCredentials::call_with_refresh`].
#[derive(Debug, thiserror::Error)]
pub enum MoocAuthFailure {
    /// Rejected and refreshing didn't recover it (refresh token itself rejected, nothing to
    /// refresh with, or the retry was rejected too). Credentials are already deleted.
    #[error("mooc authentication was rejected and could not be refreshed: {0}")]
    Permanent(#[source] Box<mooc::MoocClientError>),
    /// Anything else -- an ordinary error from `op`, or a refresh that failed only transiently.
    /// Safe to retry later; credentials are untouched.
    #[error("{0}")]
    Other(#[source] Box<mooc::MoocClientError>),
}

impl MoocAuthFailure {
    /// Whether this is an unrecoverable auth failure (credentials already
    /// deleted), as opposed to an ordinary or transient one that is safe to
    /// retry.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}

impl From<MoocAuthFailure> for LangsError {
    fn from(err: MoocAuthFailure) -> Self {
        match err {
            MoocAuthFailure::Permanent(e) | MoocAuthFailure::Other(e) => LangsError::MoocClient(e),
        }
    }
}

/// Reads and parses the credentials file under a shared read lock. The outer
/// `Result` is I/O/locking failure; the inner is a parse failure the caller
/// handles by deleting the file.
fn read_stored_shared(
    path: &Path,
) -> Result<Result<StoredCredentials, tmc_langs_util::JsonError>, LangsError> {
    let mut lock = Lock::file(path, LockOptions::Read)?;
    let guard = lock.lock()?;
    Ok(deserialize::json_from_reader(guard.get_file()))
}

/// Reads and parses the credentials file via a plain handle. Used only while an
/// exclusive lock is already held on the file by the caller.
fn read_stored(path: &Path) -> Result<StoredCredentials, LangsError> {
    let bytes = file_util::read_file(path)?;
    let stored = deserialize::json_from_slice(&bytes)
        .map_err(|e| LangsError::DeserializeCredentials(path.to_path_buf(), e))?;
    Ok(stored)
}

/// Atomic write of the wrapper JSON, creating the config directory if needed.
///
/// The bytes are written to a temp file in the same directory and then renamed
/// over the target, so a reader never observes a torn or empty file even if it
/// were to read without blocking on the lock (and, unlike a truncate-in-place
/// write, a crash mid-write can't leave the credentials empty). On unix the file
/// is restricted to `0600`: it holds an OAuth refresh token, so no other user
/// should be able to read it. Callers already holding an exclusive lock on
/// `path` use a separate handle here (advisory locking makes that safe).
fn write_stored(path: &Path, stored: &StoredCredentials) -> Result<(), LangsError> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    file_util::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(stored)?;

    // Temp file in the same directory so the final rename stays within one
    // filesystem and is therefore atomic.
    let mut temp = file_util::named_temp_file_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| tmc_langs_util::FileError::FileWrite(temp.path().to_path_buf(), e))?;
    }
    temp.write_all(&bytes)
        .map_err(|e| tmc_langs_util::FileError::FileWrite(temp.path().to_path_buf(), e))?;
    temp.flush()
        .map_err(|e| tmc_langs_util::FileError::FileWrite(temp.path().to_path_buf(), e))?;
    temp.persist(path)
        .map_err(|e| tmc_langs_util::FileError::Rename {
            from: e.file.path().to_path_buf(),
            to: path.to_path_buf(),
            source: e.error,
        })?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use mockito::{Matcher, Server};
    use oauth2::{AccessToken, EmptyExtraTokenFields, RefreshToken, basic::BasicTokenType};
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    // The credentials path is derived from `TMC_LANGS_CONFIG_DIR`, a process-wide
    // env var, so tests that set it run under a shared lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_config_dir(dir: &Path) {
        // SAFETY: all env access in these tests is serialized by ENV_LOCK.
        unsafe {
            std::env::set_var(crate::TMC_LANGS_CONFIG_DIR_VAR, dir);
        }
    }

    fn make_token(access: &str, refresh: Option<&str>, expires_in: Option<u64>) -> api::Token {
        let mut token = api::Token::new(
            AccessToken::new(access.to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        if let Some(refresh) = refresh {
            token.set_refresh_token(Some(RefreshToken::new(refresh.to_string())));
        }
        if let Some(secs) = expires_in {
            token.set_expires_in(Some(&Duration::from_secs(secs)));
        }
        token
    }

    #[test]
    fn saves_and_loads_roundtrip() {
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());

        let token = make_token("access-1", Some("refresh-1"), Some(3600));
        MoocCredentials::save("test", token).unwrap();

        let loaded = MoocCredentials::load("test").unwrap().unwrap();
        assert_eq!(loaded.token().access_token().secret(), "access-1");
        assert_eq!(
            loaded.token().refresh_token().unwrap().secret(),
            "refresh-1"
        );
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only_readable() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());

        let token = make_token("access-1", Some("refresh-1"), Some(3600));
        MoocCredentials::save("test", token).unwrap();

        let path = MoocCredentials::get_credentials_path("test").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credentials file (holds a refresh token) must be owner-only"
        );

        // A refresh rewrites the file; it must stay 0600 too.
        let mut server = Server::new();
        let _refresh = mock_refresh(&mut server, "new-access", "new-refresh");
        let expired = StoredCredentials {
            token: make_token("access-1", Some("refresh-1"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(7200),
        };
        write_stored(&path, &expired).unwrap();
        let root_url: Url = server.url().parse().unwrap();
        MoocCredentials::load_valid("test", &root_url, "test-client")
            .unwrap()
            .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "refresh must preserve 0600");
    }

    #[test]
    fn load_absent_is_none() {
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        assert!(MoocCredentials::load("test").unwrap().is_none());
    }

    #[test]
    fn corrupt_file_is_deleted() {
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        let path = MoocCredentials::get_credentials_path("test").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();

        assert!(MoocCredentials::load("test").is_err());
        assert!(!path.exists(), "corrupt credentials file should be deleted");
    }

    #[test]
    fn expiry_margin() {
        // Valid: an hour of life left.
        let fresh = StoredCredentials {
            token: make_token("a", Some("r"), Some(3600)),
            obtained_at: Utc::now(),
        };
        assert!(fresh.is_valid_with_margin(EXPIRY_MARGIN_SECS));

        // Invalid: obtained an hour ago with a one-hour lifetime -> expired.
        let expired = StoredCredentials {
            token: make_token("a", Some("r"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(3600),
        };
        assert!(!expired.is_valid_with_margin(EXPIRY_MARGIN_SECS));

        // Invalid: within the margin (30s left, 60s margin).
        let within_margin = StoredCredentials {
            token: make_token("a", Some("r"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(3570),
        };
        assert!(!within_margin.is_valid_with_margin(EXPIRY_MARGIN_SECS));

        // No lifetime info -> treated as valid.
        let no_expiry = StoredCredentials {
            token: make_token("a", Some("r"), None),
            obtained_at: Utc::now(),
        };
        assert!(no_expiry.is_valid_with_margin(EXPIRY_MARGIN_SECS));
    }

    /// Mounts a `/token` refresh endpoint returning a rotated token pair.
    fn mock_refresh(server: &mut Server, new_access: &str, new_refresh: &str) -> mockito::Mock {
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("grant_type=refresh_token".to_string()),
                Matcher::Regex("refresh_token=".to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "access_token": new_access,
                    "refresh_token": new_refresh,
                    "token_type": "bearer",
                    "expires_in": 3600,
                })
                .to_string(),
            )
            .expect(1)
            .create()
    }

    #[test]
    fn load_valid_refreshes_expired_token() {
        let _guard = env_lock();
        let mut server = Server::new();
        let refresh = mock_refresh(&mut server, "new-access", "new-refresh");

        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        // Seed an expired token with a refresh token.
        let expired = StoredCredentials {
            token: make_token("old-access", Some("old-refresh"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(7200),
        };
        let path = MoocCredentials::get_credentials_path("test").unwrap();
        write_stored(&path, &expired).unwrap();

        let root_url: Url = server.url().parse().unwrap();
        let loaded = MoocCredentials::load_valid("test", &root_url, "test-client")
            .unwrap()
            .unwrap();
        refresh.assert();
        assert_eq!(loaded.token().access_token().secret(), "new-access");
        assert_eq!(
            loaded.token().refresh_token().unwrap().secret(),
            "new-refresh"
        );
        // The rotated pair was persisted.
        let reloaded = MoocCredentials::load("test").unwrap().unwrap();
        assert_eq!(reloaded.token().access_token().secret(), "new-access");
    }

    #[test]
    fn load_valid_keeps_valid_token_without_refresh() {
        let _guard = env_lock();
        let mut server = Server::new();
        // No refresh call expected.
        let refresh = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .expect(0)
            .create();

        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        let token = make_token("still-good", Some("r"), Some(3600));
        MoocCredentials::save("test", token).unwrap();

        let root_url: Url = server.url().parse().unwrap();
        let loaded = MoocCredentials::load_valid("test", &root_url, "test-client")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.token().access_token().secret(), "still-good");
        refresh.assert();
    }

    #[test]
    fn load_valid_deletes_on_permanent_refresh_failure() {
        // A 400 + `invalid_grant` means the refresh token is permanently invalid
        // (revoked/expired): the credentials are deleted and `Ok(None)` returned
        // so the caller is sent through the login flow again.
        let _guard = env_lock();
        let mut server = Server::new();
        let _refresh = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant"}"#)
            .create();

        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        let expired = StoredCredentials {
            token: make_token("old", Some("bad-refresh"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(7200),
        };
        let path = MoocCredentials::get_credentials_path("test").unwrap();
        write_stored(&path, &expired).unwrap();

        let root_url: Url = server.url().parse().unwrap();
        let result = MoocCredentials::load_valid("test", &root_url, "test-client").unwrap();
        assert!(
            result.is_none(),
            "permanent refresh failure should yield no creds"
        );
        assert!(
            !path.exists(),
            "permanent refresh failure should delete the creds file"
        );
    }

    #[test]
    fn load_valid_keeps_credentials_on_transient_refresh_failure() {
        // A transient failure (here a 503) must NOT delete the credentials or log
        // the user out: it surfaces a retryable error, and a later refresh
        // against a healthy backend reuses the retained token.
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        let expired = StoredCredentials {
            token: make_token("old-access", Some("old-refresh"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(7200),
        };
        let path = MoocCredentials::get_credentials_path("test").unwrap();
        write_stored(&path, &expired).unwrap();

        // First attempt: the backend is flaky (503).
        let mut failing = Server::new();
        let failing_mock = failing
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(503)
            .with_body("service unavailable")
            .expect(1)
            .create();
        let failing_url: Url = failing.url().parse().unwrap();
        let result = MoocCredentials::load_valid("test", &failing_url, "test-client");
        assert!(
            result.is_err(),
            "a transient refresh failure must surface an error, not Ok(None)"
        );
        failing_mock.assert();
        assert!(
            path.exists(),
            "a transient refresh failure must keep the credentials file"
        );
        // The stored token is untouched and still carries its refresh token.
        let reloaded = MoocCredentials::load("test").unwrap().unwrap();
        assert_eq!(
            reloaded.token().refresh_token().unwrap().secret(),
            "old-refresh"
        );

        // Second attempt against a healthy backend: succeeds using the retained
        // (old) refresh token.
        let mut working = Server::new();
        let working_mock = working
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .match_body(Matcher::Regex("refresh_token=old-refresh".to_string()))
            .with_body(
                serde_json::json!({
                    "access_token": "recovered-access",
                    "refresh_token": "recovered-refresh",
                    "token_type": "bearer",
                    "expires_in": 3600,
                })
                .to_string(),
            )
            .expect(1)
            .create();
        let working_url: Url = working.url().parse().unwrap();
        let loaded = MoocCredentials::load_valid("test", &working_url, "test-client")
            .unwrap()
            .unwrap();
        working_mock.assert();
        assert_eq!(loaded.token().access_token().secret(), "recovered-access");
        assert!(path.exists());
    }

    #[test]
    fn concurrent_load_valid_refreshes_exactly_once() {
        let _guard = env_lock();
        let mut server = Server::new();
        // The mock asserts it is hit exactly once across all threads.
        let refresh = mock_refresh(&mut server, "shared-new-access", "shared-new-refresh");

        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        let expired = StoredCredentials {
            token: make_token("old-access", Some("old-refresh"), Some(3600)),
            obtained_at: Utc::now() - chrono::Duration::seconds(7200),
        };
        let path = MoocCredentials::get_credentials_path("test").unwrap();
        write_stored(&path, &expired).unwrap();

        let root_url: Url = server.url().parse().unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let root_url = root_url.clone();
                std::thread::spawn(move || {
                    MoocCredentials::load_valid("test", &root_url, "test-client")
                        .unwrap()
                        .unwrap()
                        .token()
                        .access_token()
                        .secret()
                        .clone()
                })
            })
            .collect();

        for handle in threads {
            let access = handle.join().unwrap();
            assert_eq!(
                access, "shared-new-access",
                "all threads end with the refreshed token"
            );
        }
        // Exactly one network refresh despite 8 concurrent loaders.
        refresh.assert();
    }
}
