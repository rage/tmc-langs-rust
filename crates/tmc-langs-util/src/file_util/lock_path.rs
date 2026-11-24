//! Maps a directory to its lock file in a central locks directory, rather than to a
//! lock file inside the directory itself.
//!
//! Accepted trade-offs: the locks dir is per OS user, so two users locking the same
//! shared directory don't exclude each other; and older tmc-langs versions lock the
//! in-directory `.tmc.lock`, which this version never takes, so a concurrently running
//! old client isn't excluded either.

use super::LockOptions;
use crate::error::FileError;
use std::{
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

/// Overrides the central locks directory, for when the default (the OS user's local
/// data dir, falling back to the temp dir) isn't writable or appropriate.
pub const LOCKS_DIR_ENV: &str = "TMC_LANGS_LOCKS_DIR";

static TEST_LOCKS_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// In-process override for test binaries that can't set `LOCKS_DIR_ENV` safely:
/// `std::env::set_var` is unsafe because it races other threads' env reads, and in a
/// shared test binary those threads are usually already running by the time a test
/// wants isolation. Takes effect once per process; later calls are no-ops.
/// `LOCKS_DIR_ENV` is unaffected and remains the escape hatch for real users.
pub fn set_test_locks_dir_override(dir: PathBuf) {
    let _ = TEST_LOCKS_DIR_OVERRIDE.set(dir);
}

fn locks_dir() -> Result<PathBuf, FileError> {
    if let Some(dir) = TEST_LOCKS_DIR_OVERRIDE.get() {
        return Ok(dir.clone());
    }

    if let Ok(dir) = std::env::var(LOCKS_DIR_ENV) {
        let dir = PathBuf::from(dir);
        super::create_dir_all(&dir)?;
        return Ok(dir);
    }

    // isolates this crate's own tests from the real user data dir; cfg(test) doesn't reach
    // crates depending on tmc-langs-util normally, hence the override above for those
    #[cfg(test)]
    {
        static TEST_LOCKS_DIR: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| {
                tempfile::tempdir().expect("failed to create tempdir for test")
            });
        Ok(TEST_LOCKS_DIR.path().to_path_buf())
    }

    #[cfg(not(test))]
    resolve_default_locks_dir(dirs::data_local_dir(), std::env::temp_dir())
}

/// Split out from `locks_dir` so tests can reach it with injected inputs; the `#[cfg(test)]`
/// short-circuit above means they never get here otherwise.
fn resolve_default_locks_dir(
    data_local_dir: Option<PathBuf>,
    temp_dir: PathBuf,
) -> Result<PathBuf, FileError> {
    // can be None (HOME/XDG unset) or unwritable (e.g. some containers); fall
    // through to the temp dir rather than failing the lock outright
    if let Some(dir) = data_local_dir {
        let dir = dir.join("tmc-langs").join("locks");
        if super::create_dir_all(&dir).is_ok() {
            prune_stale_locks_once(&dir);
            return Ok(dir);
        }
    }

    // keyed per user: /tmp is shared, so an unkeyed dir would let whichever OS user gets
    // there first own (or block) every other user's locks
    let locks_dir_name = format!("locks-{}", user_key());
    // tmc-langs 0.42 and earlier lock in `tmc-langs/locks-<user>`; staying there keeps an older
    // CLI running beside this one excluded, unless another user controls `tmc-langs`
    let legacy_parent = temp_dir.join("tmc-langs");
    let dir = if is_usable_legacy_parent(&legacy_parent) {
        legacy_parent.join(locks_dir_name)
    } else {
        temp_dir.join(format!("tmc-langs-{locks_dir_name}"))
    };
    create_private_dir(&dir)?;
    prune_stale_locks_once(&dir);
    Ok(dir)
}

/// Whether `dir` exists or could be created, belonging to this user and not writable by others.
/// Group write is tolerated: older versions created it with the default umask, which on systems
/// with per-user groups is 002.
#[cfg(unix)]
fn is_usable_legacy_parent(dir: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    if let Err(err) = std::fs::DirBuilder::new().mode(0o700).create(dir)
        && err.kind() != std::io::ErrorKind::AlreadyExists
    {
        return false;
    }
    std::fs::symlink_metadata(dir).is_ok_and(|metadata| {
        metadata.is_dir()
            && metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.mode() & 0o002 == 0
    })
}

#[cfg(not(unix))]
fn is_usable_legacy_parent(dir: &Path) -> bool {
    super::create_dir_all(dir).is_ok()
}

/// Creates `dir` readable only by this user, or accepts an existing one only if this user owns
/// it and nobody else can write to it: in a shared temp dir another user may have made it first.
#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), FileError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(FileError::DirCreate(dir.to_path_buf(), err)),
    }
    let metadata = std::fs::symlink_metadata(dir)
        .map_err(|err| FileError::DirCreate(dir.to_path_buf(), err))?;
    let is_private = metadata.is_dir()
        && metadata.uid() == nix::unistd::geteuid().as_raw()
        && metadata.mode() & 0o022 == 0;
    if is_private {
        Ok(())
    } else {
        Err(FileError::DirCreate(
            dir.to_path_buf(),
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the lock directory exists but belongs to another user or is writable by others",
            ),
        ))
    }
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), FileError> {
    super::create_dir_all(dir)
}

fn sanitize_ascii(s: &str, max_len: usize) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(max_len)
        .collect()
}

/// Best-effort stable identity for the temp-dir fallback. `data_local_dir()` above is
/// already per-user via HOME/XDG; this only matters once that's unavailable.
fn user_key() -> String {
    let env_user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    if let Some(sanitized) = env_user
        .map(|u| sanitize_ascii(&u, 32))
        .filter(|s| !s.is_empty())
    {
        return sanitized;
    }

    #[cfg(target_os = "linux")]
    if let Some(uid) = linux_real_uid() {
        return uid.to_string();
    }

    "shared".to_string()
}

// no env var and no /proc (non-Linux, stripped environment): identity is unknown, so
// two such users would still collide on this literal name
#[cfg(target_os = "linux")]
fn linux_real_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

const STALE_LOCK_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

static PRUNE_ONCE: std::sync::Once = std::sync::Once::new();

fn prune_stale_locks_once(dir: &Path) {
    PRUNE_ONCE.call_once(|| prune_stale_locks(dir));
}

/// Sweeps lock files older than `STALE_LOCK_AGE`, which otherwise accumulate one per directory
/// ever locked. Removes one only while holding it exclusively (see `remove_if_unheld`): deleting a
/// lock out from under its holder is the bug this locks dir exists to avoid.
fn prune_stale_locks(dir: &Path) {
    let Ok(shards) = std::fs::read_dir(dir) else {
        return;
    };
    for shard in shards.flatten() {
        if !shard.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(shard.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lock") {
                continue;
            }
            let is_stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > STALE_LOCK_AGE));
            if is_stale {
                remove_if_unheld(&path);
            }
        }
    }
}

#[cfg(unix)]
fn remove_if_unheld(path: &Path) {
    use file_lock::{FileLock, FileOptions};
    if let Ok(_lock) = FileLock::lock(path, false, FileOptions::new().write(true)) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
fn remove_if_unheld(path: &Path) {
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    else {
        return;
    };
    let mut lock = fd_lock::RwLock::new(file);
    if let Ok(_guard) = lock.try_write() {
        let _ = std::fs::remove_file(path);
    }
}

/// Returns the central lock file for `path`, creating the locks dir and, for the
/// *Create options, `path` itself.
pub(crate) fn central_lock_path(path: &Path, options: LockOptions) -> Result<PathBuf, FileError> {
    if matches!(options, LockOptions::ReadCreate | LockOptions::WriteCreate) {
        super::create_dir_all(path)?;
    }

    // both the hash and the suffix must come from `stable`, never the raw `path`, or two
    // spellings of one directory ("." vs absolute) map to two different lock files
    let stable = stable_key(path);
    let mut key = stable.to_string_lossy().into_owned();
    if cfg!(windows) {
        // case-insensitive filesystem, so differently-cased paths must hash the same
        key = key.to_lowercase();
    }

    let hash = blake3::hash(key.as_bytes()).to_hex();
    let file_name = match sanitized_suffix(&stable) {
        Some(suffix) => format!("{}-{suffix}.lock", &hash[..16]),
        None => format!("{}.lock", &hash[..16]),
    };
    // shards by the hash prefix (like git's object store) so the locks dir stays
    // browsable even after years of never-deleted (until pruned) lock files
    let shard_dir = locks_dir()?.join(&hash[..2]);
    super::create_dir_all(&shard_dir)?;
    Ok(shard_dir.join(file_name))
}

/// Canonicalizes the deepest existing ancestor and appends the rest lexically, so a
/// directory hashes the same before and after creation even through a symlinked parent.
fn stable_key(path: &Path) -> PathBuf {
    let absolute = normalize_absolute(path);

    let mut suffix = Vec::new();
    let mut ancestor: &Path = &absolute;
    loop {
        if let Ok(canon) = super::canonicalize(ancestor) {
            let mut key = canon;
            key.extend(suffix.into_iter().rev());
            return key;
        }
        let Some(parent) = ancestor.parent() else {
            // even the root failed to canonicalize; use the lexical path rather than fail
            break;
        };
        suffix.push(ancestor.file_name().expect("has a parent, so has a name"));
        ancestor = parent;
    }
    absolute
}

fn normalize_absolute(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    lexically_normalize(&absolute)
}

// keeps `stable_key`'s ancestor walk on the path shape a later `canonicalize` would produce
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                }
            }
            other => result.push(other),
        }
    }
    result
}

// only to make the locks dir browsable; the hash is what identifies the lock
fn sanitized_suffix(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    let sanitized = sanitize_ascii(&name, 32);
    (!sanitized.is_empty()).then_some(sanitized)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn same_directory_hashes_to_the_same_lock_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = central_lock_path(dir.path(), LockOptions::Read).unwrap();
        let b = central_lock_path(dir.path(), LockOptions::Write).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_directories_hash_to_different_lock_paths() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let a = central_lock_path(a.path(), LockOptions::Read).unwrap();
        let b = central_lock_path(b.path(), LockOptions::Read).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn lock_path_is_outside_the_locked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = central_lock_path(dir.path(), LockOptions::Read).unwrap();
        assert!(!lock_path.starts_with(dir.path()));
    }

    #[test]
    fn lock_file_lives_in_a_two_char_hex_shard_dir() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = central_lock_path(dir.path(), LockOptions::Read).unwrap();
        let shard = lock_path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(shard.len(), 2);
        assert!(shard.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn write_create_creates_missing_directory_before_hashing() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("does-not-exist-yet");
        assert!(!missing.exists());
        central_lock_path(&missing, LockOptions::WriteCreate).unwrap();
        assert!(missing.is_dir());
    }

    #[test]
    fn missing_directory_without_create_falls_back_instead_of_erroring() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("does-not-exist");
        central_lock_path(&missing, LockOptions::Read).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn key_of_missing_dir_through_symlink_matches_key_after_creation() {
        let real_parent = tempfile::tempdir().unwrap();
        let link_holder = tempfile::tempdir().unwrap();
        let link = link_holder.path().join("link");
        std::os::unix::fs::symlink(real_parent.path(), &link).unwrap();

        let missing = link.join("child");
        assert!(!missing.exists());
        let before = central_lock_path(&missing, LockOptions::Read).unwrap();

        std::fs::create_dir_all(&missing).unwrap();
        let after = central_lock_path(&missing, LockOptions::Read).unwrap();

        assert_eq!(before, after);
    }

    #[test]
    fn spelling_of_the_same_directory_hashes_to_the_same_lock_path() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = central_lock_path(dir.path(), LockOptions::Read).unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let dot = central_lock_path(Path::new("."), LockOptions::Read);
        std::env::set_current_dir(original_dir).unwrap();
        assert_eq!(canonical, dot.unwrap());

        let trailing_slash = dir.path().join("");
        assert_eq!(
            canonical,
            central_lock_path(&trailing_slash, LockOptions::Read).unwrap()
        );

        let dot_dot_terminated = dir.path().join("child").join("..");
        assert_eq!(
            canonical,
            central_lock_path(&dot_dot_terminated, LockOptions::Read).unwrap()
        );
    }

    #[test]
    fn uses_data_local_dir_when_it_is_writable() {
        let data_local = tempfile::tempdir().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = resolve_default_locks_dir(
            Some(data_local.path().to_path_buf()),
            temp_dir.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(dir, data_local.path().join("tmc-langs").join("locks"));
    }

    #[test]
    fn falls_back_to_temp_dir_when_data_local_dir_is_unavailable() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = resolve_default_locks_dir(None, temp_dir.path().to_path_buf()).unwrap();
        assert!(dir.starts_with(temp_dir.path()));
    }

    // turning the fallthrough's `.is_ok()` into a `?` would make an unwritable data dir a hard
    // error instead of a fallback, and nothing else in the suite would notice
    #[test]
    #[cfg(unix)]
    fn falls_back_to_temp_dir_when_data_local_dir_is_unwritable() {
        use std::os::unix::fs::PermissionsExt;

        let unwritable = tempfile::tempdir().unwrap();
        std::fs::set_permissions(unwritable.path(), std::fs::Permissions::from_mode(0o500))
            .unwrap();
        let temp_dir = tempfile::tempdir().unwrap();

        let result = resolve_default_locks_dir(
            Some(unwritable.path().to_path_buf()),
            temp_dir.path().to_path_buf(),
        );

        std::fs::set_permissions(unwritable.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        assert!(result.unwrap().starts_with(temp_dir.path()));
    }

    #[test]
    #[cfg(unix)]
    fn temp_dir_fallback_is_restricted_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let dir = resolve_default_locks_dir(None, temp_dir.path().to_path_buf()).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    #[cfg(unix)]
    fn temp_dir_fallback_refuses_a_directory_others_can_write_to() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp_dir.path().join("tmc-langs")).unwrap();
        let planted = temp_dir
            .path()
            .join("tmc-langs")
            .join(format!("locks-{}", user_key()));
        std::fs::create_dir(&planted).unwrap();
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(resolve_default_locks_dir(None, temp_dir.path().to_path_buf()).is_err());
    }

    // tmc-langs 0.42 locks there, and a CLI of each version may run at once
    #[test]
    fn temp_dir_fallback_shares_the_directory_older_versions_lock_in() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = resolve_default_locks_dir(None, temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(
            dir,
            temp_dir
                .path()
                .join("tmc-langs")
                .join(format!("locks-{}", user_key()))
        );
    }

    #[test]
    #[cfg(unix)]
    fn temp_dir_fallback_avoids_a_shared_parent_others_can_write_to() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let shared = temp_dir.path().join("tmc-langs");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        let dir = resolve_default_locks_dir(None, temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(
            dir,
            temp_dir
                .path()
                .join(format!("tmc-langs-locks-{}", user_key()))
        );
    }

    #[test]
    fn user_key_is_stable_within_a_process() {
        assert_eq!(user_key(), user_key());
        assert!(!user_key().is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn prune_stale_locks_removes_old_files_but_keeps_fresh_ones() {
        use std::time::SystemTime;

        let dir = tempfile::tempdir().unwrap();
        let shard = dir.path().join("ab");
        std::fs::create_dir_all(&shard).unwrap();
        let old_time = SystemTime::now()
            .checked_sub(STALE_LOCK_AGE + Duration::from_secs(60))
            .unwrap();

        let stale = shard.join("stale.lock");
        std::fs::write(&stale, []).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        let fresh = shard.join("fresh.lock");
        std::fs::write(&fresh, []).unwrap();

        prune_stale_locks(dir.path());

        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    // `file_lock` uses fcntl-based POSIX record locks, which don't conflict with a
    // second lock request from the same process (only across processes), so
    // `remove_if_unheld` skipping a held file can't be simulated in-process here; this
    // only checks it doesn't error on a file it can lock, i.e. the non-conflicting case
    #[test]
    #[cfg(unix)]
    fn remove_if_unheld_removes_an_unheld_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("some.lock");
        std::fs::write(&path, []).unwrap();
        remove_if_unheld(&path);
        assert!(!path.exists());
    }
}
