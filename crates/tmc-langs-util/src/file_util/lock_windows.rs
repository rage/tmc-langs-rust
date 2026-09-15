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
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const RETRY_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

// 5 = ERROR_ACCESS_DENIED, 32 = ERROR_SHARING_VIOLATION; both can be a transient handle held by
// e.g. an antivirus, and 5 is what a delete-pending lock file returns, the failure this module
// exists to survive. When 5 is permanent instead, RETRY_BUDGET_LEFT_MS bounds the cost.
fn is_transient_open_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ResourceBusy
    ) || matches!(err.raw_os_error(), Some(5) | Some(32))
}

// An unwritable locks dir returns ERROR_ACCESS_DENIED for every lock, and one command locks several
// directories, so a per-lock timeout alone still lets a doomed command stall for seconds.
static RETRY_BUDGET_LEFT_MS: AtomicU64 = AtomicU64::new(4_000);

fn take_retry_budget() -> bool {
    let cost = RETRY_INTERVAL.as_millis() as u64;
    RETRY_BUDGET_LEFT_MS
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
            left.checked_sub(cost)
        })
        .is_ok()
}

/// Retries `open` on a transient error (see `is_transient_open_error`) for up to
/// `RETRY_TIMEOUT`, so a file briefly held open by e.g. an antivirus or indexer
/// doesn't turn into a hard failure.
fn open_with_retry(
    path: &Path,
    mut open: impl FnMut() -> std::io::Result<File>,
) -> std::io::Result<File> {
    let start_time = Instant::now();
    loop {
        match open() {
            Ok(file) => return Ok(file),
            Err(err)
                if is_transient_open_error(&err)
                    && start_time.elapsed() < RETRY_TIMEOUT
                    && take_retry_budget() =>
            {
                log::warn!("failed to open {} ({err}), retrying", path.display());
                std::thread::sleep(RETRY_INTERVAL);
            }
            Err(err) => return Err(err),
        }
    }
}

/// Logs an escalating warning naming `path` until the caller sets the returned flag, so a wait
/// behind a wedged process shows up in the log instead of hanging silently. Observes only: the
/// caller's blocking acquire does the locking.
fn spawn_wait_watchdog(path: PathBuf) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = Arc::clone(&done);
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut last_logged = Duration::ZERO;
        while !done_clone.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            if done_clone.load(Ordering::Relaxed) {
                break;
            }
            let elapsed = start.elapsed();
            if elapsed < Duration::from_secs(30) || elapsed - last_logged < Duration::from_secs(10)
            {
                continue;
            }
            last_logged = elapsed;
            let secs = elapsed.as_secs();
            if elapsed >= Duration::from_secs(120) {
                log::error!("still waiting to lock {} after {secs}s", path.display());
            } else {
                log::warn!("still waiting to lock {} after {secs}s", path.display());
            }
        }
    });
    done
}

/// Blocks until the lock can be acquired.
#[derive(Debug)]
pub struct Lock {
    pub path: PathBuf,
    options: LockOptions,
    lock: RwLock<File>,
    /// The actual file backing the lock, for `Lock::dir`, where it differs from `path`
    /// (the directory). `None` for `Lock::file`, where `path` already is that file.
    lock_file_path: Option<PathBuf>,
}

impl Lock {
    pub fn file(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();
        let open_options = options.into_open_options();
        let file = open_with_retry(&path, || open_options.open(&path))
            .map_err(|e| FileError::FileOpen(path.clone(), e))?;
        let lock = RwLock::new(file);
        Ok(Self {
            path,
            options,
            lock,
            lock_file_path: None,
        })
    }

    pub fn dir(path: impl AsRef<Path>, options: LockOptions) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = central_lock_path(&path, options)?;

        let file = open_with_retry(&lock_path, || {
            OpenOptions::new().write(true).create(true).open(&lock_path)
        })
        .map_err(|e| FileError::FileCreate(lock_path.clone(), e))?;
        let lock = RwLock::new(file);
        Ok(Self {
            path,
            options,
            lock,
            lock_file_path: Some(lock_path),
        })
    }

    pub fn lock(&mut self) -> Result<Guard<'_>, FileError> {
        log::trace!("locking {}", self.path.display());
        let report_path: &Path = self.lock_file_path.as_deref().unwrap_or(&self.path);
        // the try_* guard drops immediately, so the blocking acquire below still does the locking;
        // this only avoids a watchdog thread per lock in the uncontended case
        let shared = matches!(self.options, LockOptions::Read | LockOptions::ReadCreate);
        let contended = if shared {
            self.lock.try_read().is_err()
        } else {
            self.lock.try_write().is_err()
        };
        let done = contended.then(|| spawn_wait_watchdog(report_path.to_path_buf()));
        let guard = if shared {
            GuardInner::FdLockRead(self.lock.read().expect("cannot fail on Windows"))
        } else {
            GuardInner::FdLockWrite(self.lock.write().expect("cannot fail on Windows"))
        };
        if let Some(done) = done {
            done.store(true, Ordering::Relaxed);
        }
        let file: &File = match &guard {
            GuardInner::FdLockRead(g) => g,
            GuardInner::FdLockWrite(g) => g,
        };
        truncate_locked_file(self.options, file, report_path)?;
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
