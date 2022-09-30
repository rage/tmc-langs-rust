//! File locking utilities on Windows.
//!
//! Windows directories can't be locked with fd-lock, so a different solution is needed.
//! Currently, regular files are locked with fd-lock, but for directories a .tmc.lock file is created.

use crate::{error::FileError, file_util::*};
use fd_lock::{RwLock, RwLockWriteGuard};
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

/// Wrapper for fd_lock::RwLock. Used to lock files/directories to prevent concurrent access
/// from multiple instances of tmc-langs.
pub struct FileLock {
    path: PathBuf,
    // this is re-set in every lock command if the target is a file
    // ideally it would be set to none when the guard is dropped, but doing so is probably not worth the trouble
    lock: Option<RwLock<File>>,
}

impl FileLock {
    pub fn new(path: PathBuf) -> Result<FileLock, FileError> {
        Ok(Self { path, lock: None })
    }

    /// Blocks until the lock can be acquired.
    /// On Windows, directories cannot be locked, so we use a lock file instead.
    pub fn lock(&mut self) -> Result<FileLockGuard, FileError> {
        log::trace!("locking {}", self.path.display());
        let start_time = Instant::now();
        let mut warning_timer = Instant::now();

        if self.path.is_file() {
            // for files, just use the path
            let file = open_file(&self.path)?;
            let lock = RwLock::new(file);
            self.lock = Some(lock);
            let lock = self.lock.as_mut().expect("set to Some before this call");
            let guard = lock.write().expect("cannot fail on Windows");
            Ok(FileLockGuard {
                _guard: LockInner::RwLockWriteGuard(guard),
                path: Cow::Borrowed(&self.path),
            })
        } else if self.path.is_dir() {
            // for directories, we'll create/open a .tmc.lock file
            let lock_path = self.path.join(".tmc.lock");
            loop {
                // try to create a new lock file
                match OpenOptions::new()
                    // needed for create_new
                    .write(true)
                    // only creates file if it exists, check and creation are atomic
                    .create_new(true)
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
                        // was able to create a new lock file
                        return Ok(FileLockGuard {
                            _guard: LockInner::LockFile(file),
                            path: Cow::Owned(lock_path),
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
        } else {
            Err(FileError::InvalidLockPath(self.path.to_path_buf()))
        }
    }
}

pub struct FileLockGuard<'a> {
    _guard: LockInner<'a>,
    path: Cow<'a, PathBuf>,
}

enum LockInner<'a> {
    LockFile(File),
    RwLockWriteGuard(RwLockWriteGuard<'a, File>),
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        log::trace!("unlocking {}", self.path.display());
    }
}

#[cfg(test)]
mod test {
    use super::*;
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
        let mut lock = FileLock::new(temp_path.to_path_buf()).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and then mutex
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let mutex = mutex.clone();

            std::thread::spawn(move || {
                // if the file lock doesn't block, the mutex lock will panic and the test will fail
                let mut lock = FileLock::new(temp_path).unwrap();
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
        let mut lock = FileLock::new(temp_path.to_path_buf()).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and mutex
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let mutex = mutex.clone();

            std::thread::spawn(move || {
                // if the file lock doesn't block, the mutex lock will panic and the test will fail
                let mut lock = FileLock::new(temp_path).unwrap();
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
        let mut lock = FileLock::new(temp.path().to_path_buf()).unwrap();
        let lock_path = temp.path().join(".tmc.lock");
        assert!(!lock_path.exists());
        let guard = lock.lock().unwrap();
        assert!(lock_path.exists());
        drop(guard);
        assert!(!lock_path.exists());
    }
}
