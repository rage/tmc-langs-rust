//! Credentials for authenticating with the Courses MOOC backend.
//!
//! Stored separately from the legacy tmc `credentials.json` in a
//! `credentials_mooc_<host>.json` file in the same config directory. The legacy
//! file holds a tmc-server password-grant token that is worthless to the mooc
//! OAuth2 device-flow model, so it is deliberately left untouched and NOT
//! migrated.
//!
//! The file name is keyed by hostname because the config directory is very often
//! a shared network home directory; see
//! [`MoocCredentials::credentials_file_name`].
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
    sync::{Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};
use tmc_langs_util::{
    FileError, deserialize,
    file_util::{self, LockOptions},
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

/// Total budget for *acquiring* the credentials lock, shared by the in-process
/// refresh mutex and the cross-process file lock.
///
/// The backend no longer grants a reuse window for a refresh token that has
/// already been redeemed, so this lock is the only thing stopping two concurrent
/// refreshes from redeeming the same token and having the whole token family
/// revoked as a suspected theft. That makes waiting for the lock mandatory rather
/// than best-effort — but a waiter must still not wait forever.
///
/// A holder keeps the lock only for the duration of its refresh, and the refresh
/// HTTP request is itself bounded by [`mooc::AUTH_REQUEST_TIMEOUT`], so a holder
/// that is merely slow releases within that. 60s leaves double that headroom, so a
/// slow-but-succeeding refresh by another process is always waited out, while a
/// holder wedged in a way the HTTP timeout cannot catch (a stopped process, a
/// thrashing machine, a lock stranded by a filesystem that reports locks it does
/// not honour) degrades to a failed, retryable command inside a minute instead of
/// hanging the editor indefinitely.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// The lock-wait budget is only meaningful if it outlasts the network call made
/// under the lock; otherwise waiters would give up on a refresh that is still
/// legitimately in flight.
const _: () = assert!(
    LOCK_TIMEOUT.as_secs() > mooc::AUTH_REQUEST_TIMEOUT.as_secs(),
    "the credentials lock-wait budget must exceed the refresh request timeout"
);

/// How long [`lock_in_process_until`] sleeps between attempts on [`REFRESH_LOCK`].
const IN_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Bounded acquisition of [`REFRESH_LOCK`]. `std::sync::Mutex` has no timed lock,
/// so poll `try_lock` until `deadline` and then report the same
/// [`FileError::LockTimeout`] the cross-process wait uses, so a caller (and the
/// user) sees one failure mode regardless of which of the two locks was stuck.
fn lock_in_process_until(
    deadline: Instant,
    path: &Path,
) -> Result<MutexGuard<'static, ()>, LangsError> {
    loop {
        match REFRESH_LOCK.try_lock() {
            Ok(guard) => return Ok(guard),
            // A previous holder panicked mid-refresh. The mutex guards `()`, so
            // there is no poisoned state to protect: take it over.
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

/// The name the credentials were stored under before they were keyed by host, and
/// the name still used when this machine's hostname cannot be determined.
const SHARED_CREDENTIALS_FILE_NAME: &str = "credentials_mooc.json";

/// Cap on the hostname component of the file name, so a long FQDN can't push the
/// path past a filesystem's name limit. Two hostnames that agree on their first
/// [`HOST_NAME_MAX_LEN`] usable characters would share a file again, which is
/// only the pre-existing behaviour and needs the two machines to be named nearly
/// identically.
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
        // `nix` is already a dependency here; this is `gethostname(2)`.
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
        // Windows sets this for every process; there is no libc `gethostname` to
        // fall back on without pulling in a Windows API crate.
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
    // All-separator leftovers ("...", "___") name nothing recognizable and would
    // collide across machines, so treat them as no hostname at all.
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

/// Moves a pre-existing shared `credentials_mooc.json` to this host's name, so a
/// user who logged in before credentials were keyed by host is not logged out by
/// the upgrade. Returns whether it was adopted.
///
/// The move is a rename rather than a copy, and that is the point: exactly one
/// machine inherits the existing session. Copying it to every host's file would
/// hand the same rotating refresh token to several machines, and the first refresh
/// would then revoke the token family for all of them — the failure this keying is
/// meant to prevent, triggered by the migration itself. The machines that lose the
/// race simply find no credentials and log in again.
///
/// Failures are logged and otherwise ignored: the worst case is that the user logs
/// in again, which is also what happens if the file was never there.
fn adopt_shared_credentials(shared: &Path, per_host: &Path) -> bool {
    if !shared.exists() {
        return false;
    }
    if per_host.exists() {
        // This host already has its own credentials, so the shared file is not
        // ours to consume; leave it for whichever machine gets there first.
        log::debug!(
            "leaving the shared {} alone, {} already exists",
            shared.display(),
            per_host.display()
        );
        return false;
    }
    // Same directory, so the rename is atomic: a machine racing us either moves
    // the file or finds it already gone.
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
    /// Keyed by hostname because the config directory frequently lives in a
    /// network home directory shared by many machines — the university lab
    /// environments this runs in are exactly that — and one credentials file
    /// shared between machines is not safe:
    ///
    /// * The backend rotates refresh tokens and no longer tolerates a refresh
    ///   token being redeemed twice; the second redemption is treated as token
    ///   theft and revokes the whole token family. Two machines refreshing the
    ///   same stored token therefore log the user out of both.
    /// * The only thing standing between them is the file lock, and on NFS
    ///   without a reachable lock daemon `fcntl` fails with `ENOLCK` and
    ///   `file_util` deliberately proceeds *unlocked* (see
    ///   [`file_util::report_locking_unavailable`]) — so on precisely the setups
    ///   where the file ends up shared, the protection against sharing it is
    ///   missing too.
    ///
    /// One file per machine removes the sharing instead of trying to arbitrate
    /// it. The cost is one device-flow login per machine, which is also the more
    /// defensible behaviour: a device-flow login is a per-device grant.
    ///
    /// Public so that tests and anything else looking for the file on disk do
    /// not have to reconstruct the name.
    pub fn credentials_file_name() -> String {
        credentials_file_name_for(host_key().as_deref())
    }

    /// This machine's credentials file, adopting a pre-existing shared one on the
    /// way if this is the first run since the file became per-host.
    ///
    /// The adoption lives here, in the one place every entry point already goes
    /// through, so no path can miss it.
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
            &self.path,
            LockOptions::Write,
            LOCK_TIMEOUT,
            |_guard| file_util::remove_file(&self.path),
        )??;
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
        file_util::with_file_lock_timeout(
            &path,
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

    /// Exclusive-lock refresh shared by the proactive and on-401 paths.
    ///
    /// Both locks are acquired against one shared [`LOCK_TIMEOUT`] deadline, so a
    /// caller's total wait is bounded by that regardless of how the wait splits
    /// between the in-process mutex and the cross-process file lock. See
    /// [`Self::refresh_holding_lock`] for the work done under them.
    fn refresh_locked(
        path: &Path,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<Option<Self>, LangsError> {
        let deadline = Instant::now() + LOCK_TIMEOUT;
        // Serialize in-process threads first (the file lock below only serializes
        // across processes).
        let _in_process = lock_in_process_until(deadline, path)?;
        // Exclusive file lock; created if missing so the lock still serializes
        // even if a racing process deleted the file after a failed refresh. Both
        // the guard and `_in_process` are RAII, so a panic or any early return
        // inside the closure releases them.
        let remaining = deadline.saturating_duration_since(Instant::now());
        file_util::with_file_lock_timeout(path, LockOptions::WriteCreate, remaining, |_guard| {
            Self::refresh_holding_lock(path, root_url, client_id, rejected_access_token)
        })?
    }

    /// The body of [`Self::refresh_locked`], run with the exclusive lock held.
    /// Re-reads the file under the lock (double-checked) before deciding to
    /// refresh.
    fn refresh_holding_lock(
        path: &Path,
        root_url: &Url,
        client_id: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<Option<Self>, LangsError> {
        if file_util::locking_unavailable() {
            // The "exclusive" lock above may not have locked anything: this
            // filesystem refused a lock at least once in this run, and
            // `file_util` fails open. The refresh below is then unprotected, and
            // the backend revokes the token family if a refresh token is redeemed
            // twice. The credentials file is per-host, so what remains at risk is
            // two commands running concurrently on this machine.
            log::warn!(
                "refreshing the mooc token without a working file lock on {} -- \
                 another command refreshing at the same time would invalidate \
                 the login and require signing in again",
                path.display()
            );
        }

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
    // Bounded wait: an exclusive holder wedged mid-refresh must not stall the hot
    // read path (and with it the whole CLI invocation) indefinitely.
    let parsed =
        file_util::with_file_lock_timeout(path, LockOptions::Read, LOCK_TIMEOUT, |guard| {
            deserialize::json_from_reader(guard.get_file())
        })?;
    Ok(parsed)
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
    use std::time::Duration;

    // The credentials path is derived from `TMC_LANGS_CONFIG_DIR`, a process-wide
    // env var, so tests that set it run under a lock shared with the crate's
    // other env-dependent tests.
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
        // Long names are capped so the path can't exceed a filesystem's limit.
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

    #[test]
    fn in_process_refresh_lock_wait_is_bounded() {
        // Holds ENV_LOCK for the same reason the refresh tests do: while
        // REFRESH_LOCK is held here, any concurrent refresh in this test binary
        // would (correctly) time out.
        let _guard = env_lock();

        // `std::sync::Mutex` is not reentrant, so `try_lock` from the holding
        // thread reports contention -- the same state a waiter sees when another
        // thread is mid-refresh, without needing a second thread or any timing.
        let _held = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // An already-expired deadline: one attempt, then give up. No sleeping, so
        // the assertion cannot race the wall clock.
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
        // The const assertion at the top of the module enforces this at compile
        // time; restate it as a test so the intent is visible where the rest of
        // the refresh behaviour is covered. A waiter that gave up before the
        // holder's HTTP request could finish would abandon a refresh that was
        // still legitimately in flight.
        assert!(
            LOCK_TIMEOUT > mooc::AUTH_REQUEST_TIMEOUT,
            "lock wait {LOCK_TIMEOUT:?} must outlast the refresh request timeout {:?}",
            mooc::AUTH_REQUEST_TIMEOUT
        );
    }

    #[test]
    fn in_process_refresh_lock_survives_a_panicking_holder() {
        // A refresh that panics must not strand the in-process lock. The guard is
        // released by unwinding, but `std::sync::Mutex` then reports the mutex as
        // poisoned; the waiter has to take it over rather than fail, since the
        // mutex guards `()` and there is no state to protect.
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

        // Deterministic: the holder thread has already been joined, so the lock is
        // free (if poisoned) and no waiting is involved.
        let recovered = lock_in_process_until(Instant::now(), Path::new("credentials_mooc.json"))
            .expect("a panicking holder must not strand the in-process refresh lock");
        drop(recovered);
    }
}
