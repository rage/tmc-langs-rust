//! File locking utilities on Unix-based platforms.

use super::LockOptions;
use crate::{
    error::FileError,
    file_util::{self, LOCK_FILE_NAME},
};
use file_lock::{FileLock, FileOptions};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

/// Blocks until the lock can be acquired.
#[derive(Debug)]
pub struct LockA {
    pub path: PathBuf,
}

impl LockA {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        Self { path }
    }
}

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
                log::error!("Failed to lock {}: {err}", path.display());
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
}

impl Drop for Lock {
    fn drop(&mut self) {
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
                    log::warn!(
                        "Failed to remove lock file {}: {err}",
                        lock_file_path.display()
                    );
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
