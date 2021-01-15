//! File locking utilities on Unix platforms.

use crate::error::FileIo;
use crate::file_util::*;
use fd_lock::{FdLock, FdLockGuard};
use std::fs::File;
use std::path::PathBuf;

#[macro_export]
macro_rules! lock {
    ( $( $path: expr ),+ ) => {
        $(
            let path_buf: PathBuf = $path.into();
            let mut fl = $crate::file_util::FileLock::new(path_buf)?;
            let _lock = fl.lock()?;
        )*
    };
}
// macros always live at the top-level, re-export here
pub use crate::lock;

/// Wrapper for fd_lock::FdLock. Used to lock files/directories to prevent concurrent access
/// from multiple instances of tmc-langs.
// TODO: should this be in file_util or in the frontend (CLI)?
pub struct FileLock {
    path: PathBuf,
    fd_lock: FdLock<File>,
}

impl FileLock {
    pub fn new(path: PathBuf) -> Result<FileLock, FileIo> {
        let file = open_file(&path)?;
        Ok(Self {
            path,
            fd_lock: FdLock::new(file),
        })
    }

    /// Blocks until the lock can be acquired.
    pub fn lock(&mut self) -> Result<FileLockGuard, FileIo> {
        log::debug!("locking {}", self.path.display());
        let path = &self.path;
        let fd_lock = &mut self.fd_lock;
        let guard = fd_lock
            .lock()
            .map_err(|e| FileIo::FdLock(path.clone(), e))?;
        log::debug!("locked {}", self.path.display());
        Ok(FileLockGuard {
            path,
            _guard: guard,
        })
    }
}

#[derive(Debug)]
pub struct FileLockGuard<'a> {
    path: &'a Path,
    _guard: FdLockGuard<'a, File>,
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        log::debug!("unlocking {}", self.path.display());
    }
}

#[cfg(test)]
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
        let guard = lock.lock().unwrap();

        let refcell = std::cell::RefCell::new(vec![]);

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let refcell = refcell.clone();

            std::thread::spawn(move || {
                let mut lock = FileLock::new(temp_path).unwrap();
                let _guard = lock.lock().unwrap();
                refcell.borrow_mut().push(1);
            })
        };

        std::thread::sleep(std::time::Duration::from_millis(100));
        refcell.borrow_mut().push(1);
        drop(guard);
        handle.join().unwrap();
    }

    #[test]
    fn locks_dir() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path();
        let mut lock = FileLock::new(temp_path.to_path_buf()).unwrap();
        let refcell = Arc::new(Mutex::new(vec![]));

        // take file lock and then refcell
        let guard = lock.lock().unwrap();
        let mut refmut = refcell.lock().unwrap();

        let handle = {
            let temp_path = temp_path.to_path_buf();
            let refcell = refcell.clone();

            std::thread::spawn(move || {
                let mut lock = FileLock::new(temp_path).unwrap();
                // block on file lock and use refcell
                let _guard = lock.lock().unwrap();
                refcell.lock().unwrap().push(1);
            })
        };

        // wait for the other thread to actually lock
        std::thread::sleep(std::time::Duration::from_millis(100));
        refmut.push(1);

        // drop refcell borrow then file lock
        drop(refmut);
        drop(guard);
        handle.join().unwrap();
    }
}
