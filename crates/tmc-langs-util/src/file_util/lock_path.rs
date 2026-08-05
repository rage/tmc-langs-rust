//! Maps a directory to its lock file in a central locks directory, rather than to a
//! lock file inside the directory itself.
//!
//! Two limitations are accepted deliberately: the locks dir is per OS user, so two
//! users locking the same directory don't exclude each other, and older tmc-langs
//! versions lock the in-directory `.tmc.lock`, so they aren't excluded either.

use super::LockOptions;
use crate::error::FileError;
use std::path::{Component, Path, PathBuf};

/// Overrides the central locks directory, for when the default (the OS user's local
/// data dir, falling back to the temp dir) isn't writable or appropriate.
pub const LOCKS_DIR_ENV: &str = "TMC_LANGS_LOCKS_DIR";

fn locks_dir() -> Result<PathBuf, FileError> {
    if let Ok(dir) = std::env::var(LOCKS_DIR_ENV) {
        let dir = PathBuf::from(dir);
        super::create_dir_all(&dir)?;
        return Ok(dir);
    }

    // isolates tests from the real user data dir without touching process env vars
    #[cfg(test)]
    {
        static TEST_LOCKS_DIR: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| {
                tempfile::tempdir().expect("failed to create tempdir for test")
            });
        Ok(TEST_LOCKS_DIR.path().to_path_buf())
    }

    #[cfg(not(test))]
    {
        // can be None (HOME/XDG unset) or unwritable; fall through rather than fail the lock
        if let Some(dir) = dirs::data_local_dir() {
            let dir = dir.join("tmc-langs").join("locks");
            if super::create_dir_all(&dir).is_ok() {
                return Ok(dir);
            }
        }

        let dir = std::env::temp_dir().join("tmc-langs").join("locks");
        super::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// Returns the central lock file for `path`, creating the locks dir and, for the
/// *Create options, `path` itself.
pub(crate) fn central_lock_path(path: &Path, options: LockOptions) -> Result<PathBuf, FileError> {
    if matches!(options, LockOptions::ReadCreate | LockOptions::WriteCreate) {
        super::create_dir_all(path)?;
    }

    // one lock file per directory however it's referenced (relative, symlink, trailing slash)
    let mut key = stable_key(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        // case-insensitive filesystem, so differently-cased paths must hash the same
        key = key.to_lowercase();
    }

    let hash = blake3::hash(key.as_bytes()).to_hex();
    let file_name = match sanitized_suffix(path) {
        Some(suffix) => format!("{}-{suffix}.lock", &hash[..16]),
        None => format!("{}.lock", &hash[..16]),
    };
    Ok(locks_dir()?.join(file_name))
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
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
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
}
