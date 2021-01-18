//! File locking utilities on Windows.
//!
//! Windows directories can't be locked with fd-lock, so a different solution is needed.
//! Currently, regular files are locked with fd-lock, but directories are opened in exclusive mode.
//! This probably means the lock needs to be used with more care; deleting a locked directory is possible on Unix but not on Windows(?).

use crate::error::FileIo;
use crate::file_util::*;
use fd_lock::{FdLock, FdLockGuard};
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use winapi::{
    shared::winerror::ERROR_SHARING_VIOLATION,
    um::{winbase::FILE_FLAG_BACKUP_SEMANTICS, winnt::GENERIC_READ},
};

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
    // this is re-set in every lock command if the target is a file
    // ideally it would be set to none when the guard is dropped, but doing so is probably not worth the trouble
    lock: Option<FdLock<File>>,
}

impl FileLock {
    pub fn new(path: PathBuf) -> Result<FileLock, FileIo> {
        Ok(Self { path, lock: None })
    }

    /// Blocks until the lock can be acquired.
    /// On Windows, directories cannot be locked, so we use a lock file instead.
    pub fn lock(&mut self) -> Result<FileLockGuard, FileIo> {
        log::debug!("locking {}", self.path.display());

        if self.path.is_file() {
            // for files, just use the path
            let file = open_file(&self.path)?;
            let lock = FdLock::new(file);
            self.lock = Some(lock);
            let guard = self.lock.as_mut().unwrap().lock().unwrap();
            Ok(FileLockGuard::File(guard, &self.path))
        } else if self.path.is_dir() {
            // for directories, we'll continuously try opening it in exclusive access mode
            loop {
                // try to create lock file
                match OpenOptions::new()
                    .access_mode(GENERIC_READ)
                    .share_mode(0) // exclusive access = fail if another process has the file open (locked)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(&self.path)
                {
                    Ok(file) => return Ok(FileLockGuard::Dir(file, &self.path)), // succeeded in "locking" the dir
                    Err(err) => {
                        let code = err.raw_os_error().unwrap();

                        if code as u32 == ERROR_SHARING_VIOLATION {
                            // file already opened in exclusive mode, wait for the other process
                            std::thread::sleep(std::time::Duration::from_secs(2));
                        } else {
                            todo!()
                        }
                    }
                }
            }
        } else {
            panic!("invalid path");
        }
    }
}

pub enum FileLockGuard<'a> {
    File(FdLockGuard<'a, File>, &'a Path), // file locked with fd-lock
    Dir(File, &'a Path),                   // directory opened in exclusive access mode
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        let path = match self {
            Self::File(_, path) => path,
            Self::Dir(_, path) => path,
        };
        log::debug!("unlocking {}", path.display());
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

    /// on windows locking the directory means we open the directory in exclusive mode
    /// this test is just to make sure it doesn't matter for the files inside the dir
    #[test]
    fn locking_dir_doesnt_lock_files() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path();
        let file_path = temp_path.join("some file");
        std::fs::write(&file_path, "some contents").unwrap();
        let mut lock = FileLock::new(temp_path.to_path_buf()).unwrap();
        let mutex = Arc::new(Mutex::new(vec![]));

        // take file lock and mutex
        let guard = lock.lock().unwrap();
        let mut mguard = mutex.try_lock().unwrap();

        let handle = {
            let file_path = file_path.clone();
            std::thread::spawn(move || {
                // we try to rewrite the file from another thread
                std::fs::write(file_path, "new contents").unwrap();
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

        assert_eq!("new contents", read_file_to_string(file_path).unwrap());
    }
}
