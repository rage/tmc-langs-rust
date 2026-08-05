//! File locking utilities on Windows.
//!
//! file-lock doesn't support Windows, so a different solution is needed.
//!
//! `Lock::dir` locks a file in a central locks directory (see `lock_path`) and never
//! deletes it. Deleting it (e.g. via FILE_FLAG_DELETE_ON_CLOSE) makes it
//! delete-pending as soon as the first of several concurrent processes closes its
//! handle, and every other process's CreateFile then fails with ERROR_ACCESS_DENIED
//! until the last handle closes. The OS releases the lock itself on process exit.

use crate::{error::FileError, file_util::*};
use fd_lock::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    borrow::Cow,
    fs::OpenOptions,
    io::ErrorKind,
    path::PathBuf,
    time::{Duration, Instant},
};

const RETRY_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

// 5 = ERROR_ACCESS_DENIED, 32 = ERROR_SHARING_VIOLATION; both can be a transient handle
// held by e.g. an antivirus or indexer
fn is_transient_open_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ResourceBusy
    ) || matches!(err.raw_os_error(), Some(5) | Some(32))
}

/// Blocks until the lock can be acquired.
#[derive(Debug)]
pub struct Lock {
    pub path: PathBuf,
    options: LockOptions,
    lock: RwLock<File>,
}

impl Lock {
    pub fn file(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let open_options = options.into_open_options();
        let file = open_options
            .open(&path)
            .map_err(|e| FileError::FileOpen(path.as_ref().to_path_buf(), e))?;
        let lock = RwLock::new(file);
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            options,
            lock,
        })
    }

    pub fn dir(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = central_lock_path(&path, options)?;

        let start_time = Instant::now();
        loop {
            match OpenOptions::new().write(true).create(true).open(&lock_path) {
                Ok(file) => {
                    let lock = RwLock::new(file);
                    return Ok(Self {
                        path,
                        options,
                        lock,
                    });
                }
                Err(err)
                    if is_transient_open_error(&err) && start_time.elapsed() < RETRY_TIMEOUT =>
                {
                    log::warn!(
                        "failed to open lock file {} ({err}), retrying",
                        lock_path.display(),
                    );
                    std::thread::sleep(RETRY_INTERVAL);
                }
                Err(err) => return Err(FileError::FileCreate(lock_path, err)),
            }
        }
    }

    pub fn lock(&mut self) -> Result<Guard<'_>, FileError> {
        log::trace!("locking {}", self.path.display());
        let guard = match self.options {
            LockOptions::Read | LockOptions::ReadCreate => {
                GuardInner::FdLockRead(self.lock.read().expect("cannot fail on Windows"))
            }
            LockOptions::Write | LockOptions::WriteCreate | LockOptions::WriteTruncate => {
                GuardInner::FdLockWrite(self.lock.write().expect("cannot fail on Windows"))
            }
        };
        if self.options.requests_truncate() {
            // truncating at open time would let two racing writers each wipe the file
            // before either holds the lock
            let file: &File = match &guard {
                GuardInner::FdLockRead(g) => g,
                GuardInner::FdLockWrite(g) => g,
            };
            file.set_len(0)
                .map_err(|e| FileError::FileWrite(self.path.clone(), e))?;
        }
        Ok(Guard {
            guard,
            path: Cow::Borrowed(&self.path),
        })
    }
}

pub struct Guard<'a> {
    guard: GuardInner<'a>,
    path: Cow<'a, PathBuf>,
}

impl Guard<'_> {
    pub fn get_file(&self) -> &File {
        match &self.guard {
            GuardInner::FdLockRead(guard) => guard,
            GuardInner::FdLockWrite(guard) => guard,
        }
    }

    pub fn get_file_mut(&mut self) -> &File {
        match &mut self.guard {
            GuardInner::FdLockRead(guard) => guard,
            GuardInner::FdLockWrite(guard) => guard,
        }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        log::trace!("unlocking {}", self.path.display());
    }
}

enum GuardInner<'a> {
    FdLockRead(RwLockReadGuard<'a, File>),
    FdLockWrite(RwLockWriteGuard<'a, File>),
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tempfile::NamedTempFile;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    #[test]
    fn locks_file() {
        init();

        let temp = NamedTempFile::new().unwrap();
        let temp_path = temp.path();
        let mut lock = Lock::file(temp_path.to_path_buf(), LockOptions::Read).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and then mutex
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let mutex = mutex.clone();

            std::thread::spawn(move || {
                // if the file lock doesn't block, the mutex lock will panic and the test will fail
                let mut lock = Lock::file(temp_path, LockOptions::Write).unwrap();
                let _guard = lock.lock().unwrap();
                mutex.try_lock().unwrap().push(1);
            })
        };

        // sleep while holding the lock to let the thread execute
        std::thread::sleep(std::time::Duration::from_millis(200));
        mguard.push(1);

        // release locks and allow the thread to proceed
        drop(mguard);
        drop(guard);
        // wait for thread, if it panicked, it tried to lock the mutex without the file lock
        handle.join().unwrap();
    }

    #[test]
    fn locks_dir() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path();
        let mut lock = Lock::dir(temp_path.to_path_buf(), LockOptions::Read).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and mutex
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let mutex = mutex.clone();

            std::thread::spawn(move || {
                // if the file lock doesn't block, the mutex lock will panic and the test will fail
                let mut lock = Lock::dir(temp_path, LockOptions::Write).unwrap();
                let _guard = lock.lock().unwrap();
                mutex.try_lock().unwrap().push(1);
            })
        };

        // release locks and allow the thread to proceed
        std::thread::sleep(std::time::Duration::from_millis(200));
        mguard.push(1);

        // release locks and allow the thread to proceed
        drop(mguard);
        drop(guard);
        // wait for thread, if it panicked, it tried to lock the mutex without the file lock
        handle.join().unwrap();
    }

    #[test]
    fn lock_file_lives_outside_the_locked_dir_and_persists() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let mut lock = Lock::dir(temp.path().to_path_buf(), LockOptions::Read).unwrap();
        let lock_path = central_lock_path(temp.path(), LockOptions::Read).unwrap();
        assert!(!lock_path.starts_with(temp.path()));
        assert!(lock_path.exists());
        let guard = lock.lock().unwrap();
        assert!(lock_path.exists());
        drop(guard);
        drop(lock);
        // deleting it here would race concurrent processes into ERROR_ACCESS_DENIED
        assert!(lock_path.exists());
    }

    #[test]
    fn write_truncate_does_not_truncate_before_lock_is_acquired() {
        init();

        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all(b"existing content").unwrap();
        let path = temp.path().to_path_buf();

        let mut lock = Lock::file(&path, LockOptions::WriteTruncate).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing content");

        let guard = lock.lock().unwrap();
        // through the guard's handle: Windows locks are mandatory, so reading the path
        // from outside while it's held fails with error 33
        assert_eq!(guard.get_file().metadata().unwrap().len(), 0);
        drop(guard);
    }
}
