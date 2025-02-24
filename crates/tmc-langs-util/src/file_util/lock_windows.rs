//! File locking utilities on Windows.
//!
//! file-lock doesn't support Windows, so a different solution is needed.

use crate::{error::FileError, file_util::*};
use fd_lock::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    borrow::Cow,
    fs::OpenOptions,
    io::ErrorKind,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use winapi::um::{
    winbase::FILE_FLAG_DELETE_ON_CLOSE,
    winnt::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_TEMPORARY},
};

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
        // for directories, we'll create/open a .tmc.lock file
        let lock_path = path.as_ref().join(LOCK_FILE_NAME);

        let start_time = Instant::now();
        let mut warning_timer = Instant::now();
        loop {
            // try to create a new lock file
            match OpenOptions::new()
                // needed for create_new
                .write(true)
                // only creates file if it exists, check and creation are atomic
                .create(true)
                // hidden, so it won't be a problem when going through the directory
                .attributes(FILE_ATTRIBUTE_HIDDEN)
                // just tells windows there's probably no point in writing this to disk;
                // this might further reduce the risk of leftover lock files
                .attributes(FILE_ATTRIBUTE_TEMPORARY)
                // windows deletes the lock file when the handle is closed = when the lock is dropped
                .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
                .open(&lock_path)
            {
                Ok(file) => {
                    // was able to create/open the lock file
                    let lock = RwLock::new(file);
                    return Ok(Self {
                        path: path.as_ref().to_path_buf(),
                        options,
                        lock,
                    });
                }
                Err(err) => {
                    if err.kind() == ErrorKind::AlreadyExists {
                        // lock file already exists, let's wait a little and try again
                        // after 30 seconds, print a warning in the logs every 10 seconds
                        // after 120 seconds, print an error in the logs every 10 seconds
                        if start_time.elapsed() > Duration::from_secs(30)
                            && warning_timer.elapsed() > Duration::from_secs(10)
                        {
                            warning_timer = Instant::now();
                            log::warn!(
                                    "The program has been waiting for lock file {} to be deleted for {} seconds,
                                    the lock file might have been left over from a previous run due to an error.",
                                    lock_path.display(),
                                    start_time.elapsed().as_secs()
                                );
                        } else if start_time.elapsed() > Duration::from_secs(120)
                            && warning_timer.elapsed() > Duration::from_secs(10)
                        {
                            warning_timer = Instant::now();
                            log::error!(
                                    "The program has been waiting for lock file {} to be deleted for {} seconds,
                                    the lock file might have been left over from a previous run due to an error.",
                                    lock_path.display(),
                                    start_time.elapsed().as_secs()
                                );
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    } else {
                        // something else went wrong, propagate error
                        return Err(FileError::FileCreate(lock_path, err));
                    }
                }
            }
        }
    }

    pub fn lock(&mut self) -> Result<Guard<'_>, FileError> {
        log::trace!("locking {}", self.path.display());
        let guard = match self.options {
            LockOptions::Read | LockOptions::ReadCreate | LockOptions::ReadTruncate => {
                GuardInner::FdLockRead(self.lock.read().expect("cannot fail on Windows"))
            }
            LockOptions::Write | LockOptions::WriteCreate | LockOptions::WriteTruncate => {
                GuardInner::FdLockWrite(self.lock.write().expect("cannot fail on Windows"))
            }
        };
        Ok(Guard {
            guard,
            path: Cow::Borrowed(&self.path),
        })
    }

    pub fn forget(self) {
        let _self = self;
        // no-op on windows
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
mod test {
    use super::*;
    use crate::file_util::LOCK_FILE_NAME;
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
    fn lock_file_is_created_and_is_deleted() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let mut lock = Lock::dir(temp.path().to_path_buf(), LockOptions::Read).unwrap();
        let lock_path = temp.path().join(LOCK_FILE_NAME);
        assert!(lock_path.exists());
        let guard = lock.lock().unwrap();
        assert!(lock_path.exists());
        drop(guard);
        drop(lock);
        assert!(!lock_path.exists());
    }
}
