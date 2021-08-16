//! File locking utilities on Unix platforms.

use crate::error::FileError;
use crate::file_util::*;
use fd_lock::{FdLock, FdLockGuard};
use std::fs::File;
use std::path::PathBuf;

/// Wrapper for fd_lock::FdLock. Used to lock files/directories to prevent concurrent access
/// from multiple instances of tmc-langs.
pub struct FileLock {
    path: PathBuf,
    fd_lock: FdLock<File>,
}

impl FileLock {
    pub fn new(path: PathBuf) -> Result<FileLock, FileError> {
        let file = open_file(&path)?;
        Ok(Self {
            path,
            fd_lock: FdLock::new(file),
        })
    }

    /// Blocks until the lock can be acquired.
    pub fn lock(&mut self) -> Result<FileLockGuard, FileError> {
        log::trace!("locking {}", self.path.display());
        let path = &self.path;
        let fd_lock = &mut self.fd_lock;
        let guard = fd_lock
            .lock()
            .map_err(|e| FileError::FdLock(path.clone(), e))?;
        log::trace!("locked {}", self.path.display());
        Ok(FileLockGuard {
            path,
            _guard: guard,
        })
    }
}

/// Guard that holds the locked file.
#[derive(Debug)]
pub struct FileLockGuard<'a> {
    path: &'a Path,
    _guard: FdLockGuard<'a, File>,
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        log::trace!("unlocking {}", self.path.display());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
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

        // take file lock and then refcell
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let mutex = mutex.clone();

            std::thread::spawn(move || {
                let mut lock = FileLock::new(temp_path).unwrap();
                let _guard = lock.lock().unwrap();
                mutex.try_lock().unwrap().push(1);
            })
        };

        std::thread::sleep(std::time::Duration::from_millis(100));
        mguard.push(1);

        drop(mguard);
        drop(guard);
        handle.join().unwrap();
    }

    #[test]
    fn locks_dir() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path();
        let mut lock = FileLock::new(temp_path.to_path_buf()).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and then refcell
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let refcell = mutex.clone();

            std::thread::spawn(move || {
                let mut lock = FileLock::new(temp_path).unwrap();
                // block on file lock and use refcell
                let _guard = lock.lock().unwrap();
                refcell.try_lock().unwrap().push(1);
            })
        };

        // wait for the other thread to actually lock
        std::thread::sleep(std::time::Duration::from_millis(100));
        mguard.push(1);

        // drop mutex guard then file lock
        drop(mguard);
        drop(guard);
        handle.join().unwrap();
    }
}
