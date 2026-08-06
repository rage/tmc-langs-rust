//! File locking utilities on Unix-based platforms.

use super::LockOptions;
use crate::{
    error::FileError,
    file_util::{self, LOCK_FILE_NAME},
};
use file_lock::{FileLock, FileOptions};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// Blocks until the lock can be acquired.
#[derive(Debug)]
pub struct Lock {
    pub path: PathBuf,
    options: LockOptions,
    lock_file_path: Option<PathBuf>,
    forget: bool,
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
            forget: false,
        })
    }

    pub fn dir(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();

        if matches!(options, LockOptions::ReadCreate | LockOptions::WriteCreate) {
            file_util::create_dir_all(&path)?;
        }

        let lock_path = path.join(LOCK_FILE_NAME);
        // first, try to create the lock file. this requires write options
        // blocking set to false so it will fail if the lock file already exists,
        // which is okay since we're not actually locking it here
        let _creator_lock = FileLock::lock(
            &lock_path,
            false,
            FileOptions::new().write(true).create(true),
        );

        Ok(Self {
            path,
            options,
            lock_file_path: Some(lock_path),
            forget: false,
        })
    }

    pub fn lock(&mut self) -> Result<Guard<'_>, FileError> {
        log::trace!("locking {}", self.path.display());
        let path = match &self.lock_file_path {
            Some(lock_file) => lock_file,
            None => &self.path,
        };
        let lock = match FileLock::lock(path, true, self.options.into_file_options()) {
            Ok(lock) => {
                log::trace!("locked {}", path.display());
                FileOrLock::Lock(lock)
            }
            Err(err) => {
                // the file locking is mostly a safeguard rather than something absolutely necessary
                // so rather than preventing the program from runningg here we'll just continue and things will probably work out
                file_util::report_locking_unavailable(path, &err);
                let file = self
                    .options
                    .into_open_options()
                    .open(&self.path)
                    .map_err(|e| FileError::FileOpen(path.to_path_buf(), e))?;
                FileOrLock::File(file)
            }
        };
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
        let path = match &self.lock_file_path {
            Some(lock_file) => lock_file,
            None => &self.path,
        };
        match FileLock::lock(path, false, self.options.into_file_options()) {
            Ok(lock) => {
                log::trace!("locked {}", path.display());
                Ok(Some(Guard {
                    lock: FileOrLock::Lock(lock),
                    path,
                }))
            }
            // `fcntl(F_SETLK)` reports an already-held lock with EAGAIN on the
            // platforms we target. POSIX also permits EACCES, but that is not
            // distinguishable from a genuine permission failure on the open, so it
            // falls through to the unsupported-locking branch below rather than
            // being retried.
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
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
                    .open(&self.path)
                    .map_err(|e| FileError::FileOpen(path.to_path_buf(), e))?;
                Ok(Some(Guard {
                    lock: FileOrLock::File(file),
                    path,
                }))
            }
        }
    }

    pub fn forget(mut self) {
        self.forget = true;
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if self.forget {
            return;
        }

        // check if we created a lock file
        if let Some(lock_file_path) = self.lock_file_path.take() {
            // try to get a write lock and delete file
            // if we can't get the lock, something else probably has it locked and we leave it there
            match FileLock::lock(
                &lock_file_path,
                false,
                FileOptions::new().read(true).write(true),
            ) {
                Ok(_) => {
                    let _ = file_util::remove_file(&lock_file_path);
                }
                Err(err) => {
                    // no need to report cases where the lockfile no longer exists
                    // (for example due to the dir being moved)
                    if !matches!(err.kind(), io::ErrorKind::NotFound) {
                        log::warn!(
                            "Failed to remove lock file {}: {err}",
                            lock_file_path.display()
                        );
                    }
                }
            }
        }
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
    fn into_file_options(self) -> FileOptions {
        match self {
            LockOptions::Read => FileOptions::new().read(true),
            LockOptions::ReadCreate => FileOptions::new().read(true).create(true),
            LockOptions::ReadTruncate => FileOptions::new().read(true).create(true).truncate(true),
            LockOptions::Write => FileOptions::new().read(true).write(true).append(true),
            LockOptions::WriteCreate => FileOptions::new()
                .read(true)
                .write(true)
                .append(true)
                .create(true),
            LockOptions::WriteTruncate => FileOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true),
        }
    }
}

#[cfg(test)]
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
}
