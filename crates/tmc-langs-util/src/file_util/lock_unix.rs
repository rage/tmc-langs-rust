//! File locking utilities on Unix-based platforms.
//!
//! Directory locks use a file in a central locks directory (see `lock_path`) and never
//! delete it, to match Windows, where deleting an in-directory lock file races
//! concurrent processes into ERROR_ACCESS_DENIED (see lock_windows.rs). The OS releases
//! the advisory lock on process exit.

use super::LockOptions;
use crate::{error::FileError, file_util};
use file_lock::{FileLock, FileOptions};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// Blocks until the lock can be acquired.
#[derive(Debug)]
pub struct Lock {
    pub path: PathBuf,
    options: LockOptions,
    lock_file_path: Option<PathBuf>,
}

impl Lock {
    pub fn file(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();

        if matches!(options, LockOptions::ReadCreate | LockOptions::WriteCreate) {
            if let Some(parent) = path.parent() {
                file_util::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            path,
            options,
            lock_file_path: None,
        })
    }

    pub fn dir(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = file_util::central_lock_path(&path, options)?;

        Ok(Self {
            path,
            options,
            lock_file_path: Some(lock_path),
        })
    }

    /// The file `fcntl` is applied to, creating it first for a dir lock.
    ///
    /// Opening the locked directory itself would give `EISDIR`, so dir locks go through the
    /// central lock file instead.
    fn ensure_lock_target(&self) -> Result<&PathBuf, FileError> {
        let path = match &self.lock_file_path {
            Some(lock_file) => lock_file,
            None => &self.path,
        };
        if self.lock_file_path.is_some() {
            // not via `FileOptions::create`, which needs write access and would turn even a
            // Read dir lock into an exclusive fcntl lock below
            OpenOptions::new()
                .write(true)
                .create(true)
                // the lock file's contents are irrelevant, but truncating it would
                // race a peer that already holds the lock
                .truncate(false)
                .open(path)
                .map_err(|e| FileError::FileCreate(path.to_path_buf(), e))?;
        }
        Ok(path)
    }

    pub fn lock(&mut self) -> Result<Guard<'_>, FileError> {
        log::trace!("locking {}", self.path.display());
        let path = self.ensure_lock_target()?;
        let lock = match FileLock::lock(path, true, self.options.into_file_options()) {
            Ok(lock) => {
                log::trace!("locked {}", path.display());
                FileOrLock::Lock(lock)
            }
            Err(err) => {
                // the file locking is mostly a safeguard rather than something absolutely necessary
                // so rather than preventing the program from running here we'll just continue and things will probably work out
                file_util::report_locking_unavailable(path, &err);
                let file = self
                    .options
                    .into_open_options()
                    .open(path)
                    .map_err(|e| FileError::FileOpen(path.to_path_buf(), e))?;
                FileOrLock::File(file)
            }
        };
        let file = match &lock {
            FileOrLock::File(f) => f,
            FileOrLock::Lock(l) => &l.file,
        };
        file_util::truncate_locked_file(self.options, file, path)?;
        Ok(Guard { lock, path })
    }

    /// A single non-blocking lock attempt. `Ok(None)` means the lock is currently
    /// held elsewhere and the caller may retry; see [`file_util::with_file_lock_timeout`]
    /// for the bounded-wait loop built on this.
    ///
    /// Note that the underlying `fcntl` lock is per-process: two `Lock`s on the
    /// same file within one process never contend with each other.
    pub fn try_lock(&mut self) -> Result<Option<Guard<'_>>, FileError> {
        log::trace!("try-locking {}", self.path.display());
        let path = self.ensure_lock_target()?;
        let lock = match FileLock::lock(path, false, self.options.into_file_options()) {
            Ok(lock) => {
                log::trace!("locked {}", path.display());
                FileOrLock::Lock(lock)
            }
            // `fcntl(F_SETLK)` reports an already-held lock with EAGAIN on the
            // platforms we target. POSIX also permits EACCES, but that is not
            // distinguishable from a genuine permission failure on the open, so it
            // falls through to the unsupported-locking branch below rather than
            // being retried.
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(err) => {
                // Locking is a safeguard rather than a hard requirement, and some
                // filesystems don't support it at all (NFS without a lock daemon
                // fails with ENOLCK), so mirror `lock` and carry on unlocked
                // instead of failing the operation. The degradation is recorded so
                // that callers relying on the lock for correctness can say so, and
                // warned about once so it is visible in the logs.
                file_util::report_locking_unavailable(path, &err);
                let file = self
                    .options
                    .into_open_options()
                    .open(path)
                    .map_err(|e| FileError::FileOpen(path.to_path_buf(), e))?;
                FileOrLock::File(file)
            }
        };
        let file = match &lock {
            FileOrLock::File(f) => f,
            FileOrLock::Lock(l) => &l.file,
        };
        file_util::truncate_locked_file(self.options, file, path)?;
        Ok(Some(Guard { lock, path }))
    }
}

#[derive(Debug)]
pub struct Guard<'a> {
    lock: FileOrLock,
    path: &'a Path,
}

impl Guard<'_> {
    pub fn get_file(&self) -> &File {
        match &self.lock {
            FileOrLock::File(f) => f,
            FileOrLock::Lock(l) => &l.file,
        }
    }

    pub fn get_file_mut(&mut self) -> &mut File {
        match &mut self.lock {
            FileOrLock::File(f) => f,
            FileOrLock::Lock(l) => &mut l.file,
        }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        log::trace!("unlocking {}", self.path.display())
    }
}

#[derive(Debug)]
enum FileOrLock {
    File(File),
    Lock(FileLock),
}

impl LockOptions {
    // no truncate: `file_lock` opens the file before locking it, so `Lock::lock`
    // truncates instead
    fn into_file_options(self) -> FileOptions {
        match self {
            LockOptions::Read => FileOptions::new().read(true),
            LockOptions::ReadCreate => FileOptions::new().read(true).create(true),
            LockOptions::Write => FileOptions::new().read(true).write(true).append(true),
            LockOptions::WriteCreate => FileOptions::new()
                .read(true)
                .write(true)
                .append(true)
                .create(true),
            LockOptions::WriteTruncate => FileOptions::new().read(true).write(true).create(true),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn can_lock_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let _lock = Lock::file(file.path(), LockOptions::Read).unwrap();
    }

    #[test]
    fn can_lock_dir() {
        let dir = tempfile::tempdir().unwrap();
        let _lock = Lock::dir(dir.path(), LockOptions::Read).unwrap();
    }

    #[test]
    fn can_delete_locked_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let _lock = Lock::file(file.path(), LockOptions::Read).unwrap();
        let _delete_lock = Lock::file(file.path(), LockOptions::Write).unwrap();
        file_util::remove_file(file.path()).unwrap();
    }

    #[test]
    fn truncate_options_do_not_truncate_on_open() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"existing content").unwrap();

        let _opened_via_file_options = LockOptions::WriteTruncate
            .into_file_options()
            .open(file.path())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "existing content"
        );

        let _opened_via_open_options = LockOptions::WriteTruncate
            .into_open_options()
            .open(file.path())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "existing content"
        );
    }

    #[test]
    fn write_truncate_truncates_only_once_locked() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"existing content").unwrap();

        let mut lock = Lock::file(file.path(), LockOptions::WriteTruncate).unwrap();
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "existing content"
        );

        let guard = lock.lock().unwrap();
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "");
        drop(guard);
    }
}
