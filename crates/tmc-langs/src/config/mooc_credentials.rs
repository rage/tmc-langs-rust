//! Credentials for authenticating with the Courses MOOC backend.
//!
//! Stored in `credentials_mooc_<host>.json`, separate from the legacy tmc
//! `credentials.json`, whose password-grant token is useless to the OAuth2
//! device flow and is deliberately not migrated. The name is keyed by hostname
//! because the config directory is often a shared network home; see
//! [`MoocCredentials::credentials_file_name`].
//!
//! `obtained_at` is stored beside the token because the `oauth2` token carries
//! only a relative `expires_in`.

use crate::error::LangsError;
use chrono::{DateTime, Utc};
use oauth2::TokenResponse;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};
use tmc_langs_util::{
    FileError, deserialize,
    file_util::{self, LockOptions},
};
use tmc_mooc_client::{self as mooc, api};
use url::Url;

/// Refresh proactively once the access token has less than this many seconds left.
const EXPIRY_MARGIN_SECS: i64 = 60;

/// Serializes refreshes between threads of this process; the file lock is
/// process-scoped and does not.
static REFRESH_LOCK: Mutex<()> = Mutex::new(());

/// Total budget for *acquiring* the credentials lock, shared by the in-process
/// mutex and the file lock.
///
/// The backend revokes the token family if a refresh token is redeemed twice, so
/// this lock is what makes concurrent refreshes safe and waiting is mandatory.
/// It must still be bounded: a holder wedged beyond what the HTTP timeout
/// ([`mooc::AUTH_REQUEST_TIMEOUT`]) can catch fails the command, retryably,
/// instead of hanging the editor.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// Waiters must outlast the refresh request made under the lock, or they abandon
/// a refresh that is still legitimately in flight.
const _: () = assert!(
    LOCK_TIMEOUT.as_secs() > mooc::AUTH_REQUEST_TIMEOUT.as_secs(),
    "the credentials lock-wait budget must exceed the refresh request timeout"
);

/// Sleep between [`lock_in_process_until`] attempts.
const IN_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Acquires [`REFRESH_LOCK`] by polling until `deadline`, since `std::sync::Mutex`
/// has no timed lock. Times out with the same [`FileError::LockTimeout`] as the
/// file lock.
fn lock_in_process_until(
    deadline: Instant,
    path: &Path,
) -> Result<MutexGuard<'static, ()>, LangsError> {
    loop {
        match REFRESH_LOCK.try_lock() {
            Ok(guard) => return Ok(guard),
            // Holder panicked; the mutex guards `()`, so nothing is poisoned.
            Err(TryLockError::Poisoned(e)) => return Ok(e.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        if Instant::now() >= deadline {
            log::warn!(
                "Gave up waiting for another thread's mooc token refresh to finish ({})",
                path.display()
            );
            return Err(LangsError::FileError(FileError::LockTimeout {
                path: path.to_path_buf(),
                timeout: LOCK_TIMEOUT,
            }));
        }
        std::thread::sleep(IN_PROCESS_POLL_INTERVAL);
    }
}

/// The pre-host-keying file name, still used when the hostname is unknown.
const SHARED_CREDENTIALS_FILE_NAME: &str = "credentials_mooc.json";

/// Cap on the hostname component of the file name, so a long FQDN can't exceed a
/// filesystem's name limit. Hostnames sharing this prefix share a file.
const HOST_NAME_MAX_LEN: usize = 64;

/// This machine's hostname, reduced to characters that are safe in a file name:
/// lowercased, with anything outside `[a-z0-9._-]` replaced by `_`. `None` when
/// the hostname is unavailable or reduces to nothing usable.
fn host_key() -> Option<String> {
    sanitize_host(&raw_host_name()?)
}

/// The unsanitized hostname as the OS reports it.
fn raw_host_name() -> Option<String> {
    #[cfg(unix)]
    {
        match nix::unistd::gethostname() {
            Ok(host) => Some(host.to_string_lossy().into_owned()),
            Err(e) => {
                log::warn!("Failed to read this machine's hostname: {e}");
                None
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Avoids pulling in a Windows API crate for `GetComputerName`.
        std::env::var("COMPUTERNAME").ok()
    }
}

/// See [`host_key`].
fn sanitize_host(host: &str) -> Option<String> {
    let sanitized: String = host
        .trim()
        .to_lowercase()
        .chars()
        .take(HOST_NAME_MAX_LEN)
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    // All-separator names ("...", "___") would collide across machines.
    if sanitized.contains(|c: char| c.is_ascii_alphanumeric()) {
        Some(sanitized)
    } else {
        None
    }
}

/// The credentials file name for a given host key, or the shared name when there
/// is none. See [`MoocCredentials::credentials_file_name`].
fn credentials_file_name_for(host_key: Option<&str>) -> String {
    match host_key {
        Some(host) => format!("credentials_mooc_{host}.json"),
        None => SHARED_CREDENTIALS_FILE_NAME.to_string(),
    }
}

/// Moves a pre-existing shared `credentials_mooc.json` to this host's name, so the
/// upgrade doesn't log the user out. Returns whether it was adopted.
///
/// Renames rather than copies so exactly one machine inherits the session:
/// copies would share one rotating refresh token, and the first refresh would
/// revoke the family for all of them. Losers of the race log in again, which is
/// also the worst case of a failure, so failures are only logged.
fn adopt_shared_credentials(shared: &Path, per_host: &Path) -> bool {
    if !shared.exists() {
        return false;
    }
    if per_host.exists() {
        // Not ours to consume; leave it for a machine that has no credentials.
        log::debug!(
            "leaving the shared {} alone, {} already exists",
            shared.display(),
            per_host.display()
        );
        return false;
    }
    // Same directory, so the rename is atomic: a racing machine finds it gone.
    match file_util::rename(shared, per_host) {
        Ok(()) => {
            log::info!(
                "adopted the previously shared mooc credentials {} as {}",
                shared.display(),
                per_host.display()
            );
            true
        }
        Err(e) => {
            log::debug!(
                "did not adopt the shared mooc credentials {}: {e}",
                shared.display()
            );
            false
        }
    }
}

/// The file that serializes access to the credentials at `path`.
///
/// The credentials file can't be the lock: it is replaced by rename, so a lock on
/// it would cover an unlinked inode, and on Windows the locking handle would
/// block both the rename and reads.
fn lock_file_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

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
    /// The name of the file this machine stores its mooc credentials in, e.g.
    /// `credentials_mooc_lab-042.json`.
    ///
    /// Keyed by hostname because the config directory is often a shared network
    /// home, and one credentials file shared between machines is unsafe: the
    /// backend treats a refresh token redeemed twice as theft and revokes the
    /// family, and on NFS without a lock daemon `fcntl` fails with `ENOLCK` and
    /// `file_util` proceeds *unlocked* (see
    /// [`file_util::report_locking_unavailable`]). One file per machine avoids the
    /// sharing at the cost of one device-flow login per machine.
    pub fn credentials_file_name() -> String {
        credentials_file_name_for(host_key().as_deref())
    }

    /// This machine's credentials file, adopting a pre-existing shared one on the
    /// first run since the file became per-host. Every entry point goes through
    /// here, so none can miss the adoption.
    fn get_credentials_path(client_name: &str) -> Result<PathBuf, LangsError> {
        let dir = super::get_tmc_dir(client_name)?;
        let path = dir.join(Self::credentials_file_name());
        let shared = dir.join(SHARED_CREDENTIALS_FILE_NAME);
        if path != shared {
            adopt_shared_credentials(&shared, &path);
        }
        Ok(path)
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
        file_util::with_file_lock_timeout(
            lock_file_path(&self.path),
            LockOptions::WriteCreate,
            LOCK_TIMEOUT,
            |_guard| file_util::remove_file(&self.path),
        )??;
        Ok(())
    }

    /// Persists a freshly obtained token, stamping it with the current time.
    ///
    /// Writes under the exclusive file lock, like [`Self::refresh_locked`]; see
    /// [`write_stored`] for atomicity.
    pub fn save(client_name: &str, token: api::Token) -> Result<(), LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        let stored = StoredCredentials {
            token,
            obtained_at: Utc::now(),
        };
        // Serializes against a concurrent refresh or `save`.
        file_util::with_file_lock_timeout(
            lock_file_path(&path),
            LockOptions::WriteCreate,
            LOCK_TIMEOUT,
            |_guard| write_stored(&path, &stored),
        )?
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
    /// A still-valid token takes a shared read lock only. Otherwise the file is
    /// re-read under an exclusive lock, so N racing refreshers collapse to one
    /// network refresh.
    ///
    /// A *permanent* failure (refresh token rejected as invalid/revoked/expired)
    /// deletes the credentials and returns `Ok(None)`, the "not logged in" path.
    /// A *transient* one (connection error, timeout, 5xx, unparseable response)
    /// leaves the file untouched and returns `Err`, so the user can retry without
    /// being logged out.
    pub fn load_valid(
        client_name: &str,
        root_url: &Url,
        client_id: &str,
    ) -> Result<Option<Self>, LangsError> {
        let path = Self::get_credentials_path(client_name)?;
        if !path.exists() {
            return Ok(None);
        }

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

        Self::refresh_locked(&path, root_url, client_id, None)
    }

    /// After a 401, refreshes the token once under an exclusive lock so the caller
    /// can retry. If another process already rotated the stored token, that one is
    /// returned without a refresh. `Ok(None)` (file deleted) means there was
    /// nothing to refresh with or the refresh token was permanently rejected; a
    /// transient failure returns `Err` and keeps the file.
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

    /// Exclusive-lock refresh shared by the proactive and on-401 paths.
    ///
    /// Both locks share one [`LOCK_TIMEOUT`] deadline, so the total wait is bounded
    /// however it splits between them. See [`Self::refresh_holding_lock`].
    fn refresh_locked(
        path: &Path,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<Option<Self>, LangsError> {
        let deadline = Instant::now() + LOCK_TIMEOUT;
        // The file lock below doesn't serialize threads.
        let _in_process = lock_in_process_until(deadline, path)?;
        // Both locks are RAII: panics and early returns release them.
        let remaining = deadline.saturating_duration_since(Instant::now());
        file_util::with_file_lock_timeout(
            lock_file_path(path),
            LockOptions::WriteCreate,
            remaining,
            |_guard| Self::refresh_holding_lock(path, root_url, client_id, rejected_access_token),
        )?
    }

    /// The body of [`Self::refresh_locked`], run with the exclusive lock held.
    fn refresh_holding_lock(
        path: &Path,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<Option<Self>, LangsError> {
        if file_util::locking_unavailable() {
            // `file_util` fails open, so the lock above may not have locked
            // anything; only concurrent commands on this machine remain at risk
            // (the file is per-host).
            log::warn!(
                "refreshing the mooc token without a working file lock on {} -- \
                 another command refreshing at the same time would invalidate \
                 the login and require signing in again",
                path.display()
            );
        }

        // Double-checked re-read under the lock.
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
            // On-401: a differing stored token was already rotated by another refresh.
            Some(rejected) => {
                if stored.token.access_token().secret() != rejected {
                    return Ok(Some(Self {
                        path: path.to_path_buf(),
                        stored,
                    }));
                }
            }
            // Proactive: another process may have refreshed while we waited.
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
            // Permanent: revoked or expired; send the user through login again.
            Err(e) if matches!(*e, mooc::MoocClientError::RefreshTokenRejected { .. }) => {
                log::warn!("mooc refresh token rejected ({e}); deleting credentials");
                file_util::remove_file(path)?;
                Ok(None)
            }
            // Transient (connection error, timeout, 5xx, unparseable response):
            // keep the credentials so a retry can succeed.
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
    /// Apply per network call, not around a multi-call subcommand: a 401 on
    /// request N must not repeat requests `1..N-1`, which is wrong for
    /// non-idempotent ones like submitting. See [`MoocAuthFailure`].
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

        // No token was ever set on this client: nothing to refresh with.
        let Some(rejected_access_token) = client.access_token() else {
            return Err(MoocAuthFailure::Permanent(err));
        };

        // Retry once if the refresh call itself fails transiently.
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
            // `refresh_after_401` already deleted the credentials.
            Ok(None) => Err(MoocAuthFailure::Permanent(err)),
            // Transient twice in a row; credentials untouched, so a later call can retry.
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

/// The parameters [`MoocCredentials::call_with_refresh`] needs, bundled so
/// multi-call functions can thread one value.
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
    // Bounded: a holder wedged mid-refresh must not stall the whole CLI invocation.
    let lock_path = lock_file_path(path);
    // `ReadCreate` can't stand in for this: on unix it opens without write
    // access, which fails and leaves the read unlocked.
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| FileError::FileCreate(lock_path.clone(), e))?;
    let bytes = file_util::with_file_lock_timeout(
        &lock_path,
        LockOptions::Read,
        LOCK_TIMEOUT,
        |_guard| file_util::read_file(path),
    )??;
    Ok(deserialize::json_from_slice(&bytes))
}

/// Reads and parses the credentials file. Used only while the caller holds the
/// exclusive lock on [`lock_file_path`].
fn read_stored(path: &Path) -> Result<StoredCredentials, LangsError> {
    let bytes = file_util::read_file(path)?;
    let stored = deserialize::json_from_slice(&bytes)
        .map_err(|e| LangsError::DeserializeCredentials(path.to_path_buf(), e))?;
    Ok(stored)
}

/// Atomic write of the wrapper JSON, creating the config directory if needed.
///
/// Writes a temp file and renames it over the target, so readers never see a
/// torn file and a crash can't leave it empty. On unix the file is `0600` since it
/// holds a refresh token. The caller must hold the exclusive lock on
/// [`lock_file_path`].
fn write_stored(path: &Path, stored: &StoredCredentials) -> Result<(), LangsError> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    file_util::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(stored)?;

    // Same directory, so the rename stays on one filesystem and is atomic.
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
    use std::time::Duration;

    // `TMC_LANGS_CONFIG_DIR` is process-wide, so tests setting it share a lock
    // with the crate's other env-dependent tests.
    use crate::config::env_lock;

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
    fn hostname_is_sanitized_into_a_file_name_component() {
        assert_eq!(sanitize_host("lab-042").as_deref(), Some("lab-042"));
        assert_eq!(
            sanitize_host("LAB-042.cs.helsinki.fi").as_deref(),
            Some("lab-042.cs.helsinki.fi")
        );
        // Nothing from the hostname may steer the path out of the config dir or
        // introduce a separator.
        assert_eq!(
            sanitize_host("../../etc/passwd").as_deref(),
            Some(".._.._etc_passwd")
        );
        assert_eq!(sanitize_host("desk top").as_deref(), Some("desk_top"));
        // Nothing usable: fall back to the shared name, i.e. the old behaviour.
        assert_eq!(sanitize_host(""), None);
        assert_eq!(sanitize_host("   "), None);
        assert_eq!(sanitize_host("///"), None);
        let long = "a".repeat(HOST_NAME_MAX_LEN + 10);
        assert_eq!(sanitize_host(&long).unwrap().len(), HOST_NAME_MAX_LEN);
    }

    #[test]
    fn credentials_file_name_is_keyed_by_host() {
        assert_eq!(
            credentials_file_name_for(Some("lab-042")),
            "credentials_mooc_lab-042.json"
        );
        // A per-host name can never collide with the shared name an older version
        // wrote, so adoption always has a distinct target.
        assert_ne!(
            credentials_file_name_for(Some("lab-042")),
            SHARED_CREDENTIALS_FILE_NAME
        );
        assert_eq!(
            credentials_file_name_for(None),
            SHARED_CREDENTIALS_FILE_NAME
        );
    }

    #[test]
    fn credentials_path_is_this_machines_file() {
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());

        let path = MoocCredentials::get_credentials_path("test").unwrap();
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            MoocCredentials::credentials_file_name()
        );
        // Guarded rather than asserted unconditionally: a machine with no readable
        // hostname legitimately keeps using the shared name.
        if host_key().is_some() {
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("credentials_mooc_"),
                "a machine with a hostname must not use the shared file name"
            );
        }
    }

    #[test]
    fn only_one_host_adopts_a_previously_shared_credentials_file() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join(SHARED_CREDENTIALS_FILE_NAME);
        std::fs::write(&shared, b"{}").unwrap();
        let first = dir.path().join(credentials_file_name_for(Some("host-a")));
        let second = dir.path().join(credentials_file_name_for(Some("host-b")));

        assert!(
            adopt_shared_credentials(&shared, &first),
            "an existing shared file must be adopted rather than ignored, so the \
             user isn't logged out by the upgrade"
        );
        assert!(first.exists());
        assert!(
            !shared.exists(),
            "the shared file must be moved, not copied: two hosts holding the same \
             rotating refresh token would revoke each other's session on refresh"
        );

        assert!(
            !adopt_shared_credentials(&shared, &second),
            "a second host has nothing to adopt"
        );
        assert!(!second.exists(), "and must not receive a copy of the token");
    }

    #[test]
    fn an_existing_per_host_file_is_not_replaced_by_a_shared_one() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join(SHARED_CREDENTIALS_FILE_NAME);
        let per_host = dir.path().join(credentials_file_name_for(Some("host-a")));
        std::fs::write(&shared, b"shared").unwrap();
        std::fs::write(&per_host, b"mine").unwrap();

        assert!(!adopt_shared_credentials(&shared, &per_host));
        assert_eq!(
            std::fs::read(&per_host).unwrap(),
            b"mine",
            "this host's own credentials must win"
        );
        assert!(
            shared.exists(),
            "and the shared file is left for whichever machine has none yet"
        );
    }

    #[test]
    fn a_shared_credentials_file_still_logs_this_machine_in() {
        let _guard = env_lock();
        let config_dir = tempfile::tempdir().unwrap();
        set_config_dir(config_dir.path());
        // Only meaningful where the file name actually differs from the shared one.
        if host_key().is_none() {
            return;
        }

        // What an install from before the file was keyed by host left behind.
        let dir = crate::config::get_tmc_dir("test").unwrap();
        let shared = dir.join(SHARED_CREDENTIALS_FILE_NAME);
        let stored = StoredCredentials {
            token: make_token("shared-access", Some("shared-refresh"), Some(3600)),
            obtained_at: Utc::now(),
        };
        write_stored(&shared, &stored).unwrap();

        let loaded = MoocCredentials::load("test")
            .unwrap()
            .expect("an upgrade must not log the user out");
        assert_eq!(loaded.token().access_token().secret(), "shared-access");
        assert!(dir.join(MoocCredentials::credentials_file_name()).exists());
        assert!(!shared.exists(), "the shared file is consumed by this host");
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
        // 400 `invalid_grant` is permanent: credentials deleted, `Ok(None)` returned.
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
        // A transient failure (503) must keep the credentials; a later refresh
        // reuses the retained token.
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
        refresh.assert();
    }

    #[test]
    fn in_process_refresh_lock_wait_is_bounded() {
        // ENV_LOCK keeps other tests' refreshes from timing out on the held REFRESH_LOCK.
        let _guard = env_lock();

        // Not reentrant: `try_lock` from the holder sees contention, as a waiter would.
        let _held = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Expired deadline: one attempt, no sleeping, so no wall-clock race.
        let err = lock_in_process_until(Instant::now(), Path::new("credentials_mooc.json"))
            .expect_err("a held in-process refresh lock must not be waited on forever");
        assert!(
            matches!(
                err,
                LangsError::FileError(FileError::LockTimeout { timeout, .. })
                    if timeout == LOCK_TIMEOUT
            ),
            "expected a lock timeout reporting the configured budget, got {err:?}"
        );
    }

    #[test]
    fn lock_wait_outlasts_the_refresh_request_timeout() {
        // Also enforced by the module's const assertion.
        assert!(
            LOCK_TIMEOUT > mooc::AUTH_REQUEST_TIMEOUT,
            "lock wait {LOCK_TIMEOUT:?} must outlast the refresh request timeout {:?}",
            mooc::AUTH_REQUEST_TIMEOUT
        );
    }

    #[test]
    fn in_process_refresh_lock_survives_a_panicking_holder() {
        // A panicking refresh poisons the mutex; waiters must take it over since it
        // guards `()`.
        let _guard = env_lock();

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panicked = std::thread::spawn(|| {
            let _held = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            panic!("simulated failure mid-refresh");
        })
        .join();
        std::panic::set_hook(hook);
        assert!(panicked.is_err(), "the holder should have panicked");

        // The holder is joined, so no waiting is involved.
        let recovered = lock_in_process_until(Instant::now(), Path::new("credentials_mooc.json"))
            .expect("a panicking holder must not strand the in-process refresh lock");
        drop(recovered);
    }
}
