//! Various utility functions, primarily wrapping the standard library's IO and filesystem functions

#[cfg(unix)]
mod lock_unix;
#[cfg(windows)]
mod lock_windows;

use crate::error::FileError;
#[cfg(unix)]
pub use lock_unix::*;
#[cfg(windows)]
pub use lock_windows::*;
use std::{
    fmt::Display,
    fs::{self, File, OpenOptions, ReadDir},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

pub const LOCK_FILE_NAME: &str = ".tmc.lock";

#[derive(Debug, Clone, Copy)]
pub enum LockOptions {
    /// Shared read lock
    Read,
    /// Shared read lock, create file if it doesn't exist instead of erroring (including intermediate directories)
    ReadCreate,
    /// Shared write lock, create file if it doesn't exist, truncate if it does
    ReadTruncate,
    /// Exclusive write lock
    Write,
    /// Exclusive write lock, create file if it doesn't exist instead of erroring (including intermediate directories)
    WriteCreate,
    /// Exclusive write lock, create file if it doesn't exist, truncate if it does
    WriteTruncate,
}

impl LockOptions {
    fn into_open_options(self) -> OpenOptions {
        let mut opts = OpenOptions::new();
        match self {
            Self::Read => opts.read(true),
            // create requires write
            Self::ReadCreate => opts.read(true).write(true).create(true),
            // truncate requires write
            Self::ReadTruncate => opts.write(true).create(true).truncate(true),
            Self::Write => opts.write(true),
            Self::WriteCreate => opts.write(true).create(true),
            Self::WriteTruncate => opts.write(true).create(true).truncate(true),
        };
        opts
    }
}

/// Set the first time a lock attempt is answered by the OS with "locking is not
/// supported here" and the caller carries on unlocked. See
/// [`report_locking_unavailable`].
static LOCKING_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// Whether locking has degraded to running *unlocked* at any point in this
/// process because the operating system refused to lock a file.
///
/// Callers that rely on a lock for correctness rather than tidiness (the mooc
/// credentials refresh is the one that matters: two unserialized refreshes redeem
/// the same OAuth refresh token and the backend revokes the session as suspected
/// theft) can use this to warn about the weaker guarantee they are now running
/// under. Sticky and process-wide on purpose: the condition is a property of the
/// filesystem, not of one attempt, and every lock taken afterwards is suspect too.
pub fn locking_unavailable() -> bool {
    LOCKING_UNAVAILABLE.load(Ordering::Relaxed)
}

/// Records that a lock could not be taken and that the caller is proceeding
/// *without* it, warning about it once per process. Returns whether this was the
/// first such report, which is also what decides whether the warning was emitted.
///
/// The fail-open behaviour is deliberate and long-standing — some filesystems do
/// not support locking at all, and refusing to work on them would be worse — but
/// it silently changes the safety properties of everything built on the lock, so
/// it must not go unmentioned in the logs. The usual cause is a home or config
/// directory on NFS with no `rpc.lockd`/NLM lock daemon reachable, where `fcntl`
/// fails with `ENOLCK`.
pub fn report_locking_unavailable(path: &Path, err: &dyn Display) -> bool {
    if LOCKING_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        // Already warned. Still record the individual occurrence, at a level that
        // cannot drown out the rest of the log if every lock in a run fails.
        log::debug!("Failed to lock {} again: {err}", path.display());
        return false;
    }
    log::warn!(
        "Failed to lock {}: {err}. Continuing WITHOUT the lock, so operations \
         that rely on it are no longer serialized between processes. This \
         usually means the filesystem does not support file locking -- most \
         often a home or config directory on NFS with no lock daemon \
         (rpc.lockd/NLM) reachable. Concurrent commands can then corrupt each \
         other's writes, and two of them refreshing the login token at once \
         will invalidate it.",
        path.display()
    );
    true
}

/// How long [`with_file_lock_timeout`] sleeps between acquisition attempts. Short
/// enough that an uncontended-again lock is picked up promptly, long enough that
/// polling for the full timeout costs a negligible number of wakeups.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Runs `f` while holding a lock on the file at `path`, giving up with
/// [`FileError::LockTimeout`] if the lock cannot be taken within `timeout`.
///
/// [`Lock::lock`] blocks indefinitely, so a process that wedges while holding an
/// exclusive lock stalls every other process for as long as it stays wedged. This
/// variant polls a non-blocking acquisition against a deadline instead, turning
/// that into a failed command the caller can retry rather than a hung one.
///
/// `f` runs with the lock held and its return value is passed through. The
/// closure form (rather than returning the guard) keeps the platform guard's
/// borrow of the internal [`Lock`] confined to a single loop iteration, which a
/// guard-returning signature cannot express.
///
/// Do not use with the truncating [`LockOptions`] variants: the file is reopened
/// on every attempt, so a contended lock would truncate it repeatedly.
pub fn with_file_lock_timeout<T>(
    path: impl AsRef<Path>,
    options: LockOptions,
    timeout: Duration,
    f: impl FnOnce(&mut Guard<'_>) -> T,
) -> Result<T, FileError> {
    let path = path.as_ref();
    let deadline = Instant::now() + timeout;
    loop {
        let mut lock = Lock::file(path, options)?;
        if let Some(mut guard) = lock.try_lock()? {
            return Ok(f(&mut guard));
        }
        // Held by someone else. One attempt always happens before the deadline is
        // checked, so a zero timeout still degrades to a plain try-lock.
        if Instant::now() >= deadline {
            log::warn!(
                "Gave up after {timeout:?} waiting for a lock on {}",
                path.display()
            );
            return Err(FileError::LockTimeout {
                path: path.to_path_buf(),
                timeout,
            });
        }
        std::thread::sleep(LOCK_POLL_INTERVAL);
    }
}

pub fn temp_file() -> Result<File, FileError> {
    tempfile::tempfile().map_err(FileError::TempFile)
}

pub fn named_temp_file() -> Result<NamedTempFile, FileError> {
    tempfile::NamedTempFile::new().map_err(FileError::TempFile)
}

pub fn named_temp_file_in(path: &Path) -> Result<NamedTempFile, FileError> {
    tempfile::NamedTempFile::new_in(path).map_err(FileError::TempFile)
}

pub fn open_file(path: impl AsRef<Path>) -> Result<File, FileError> {
    let path = path.as_ref();
    File::open(path).map_err(|e| FileError::FileOpen(path.to_path_buf(), e))
}

pub fn read_reader<R: Read>(mut reader: R) -> Result<Vec<u8>, FileError> {
    let mut bytes = vec![];
    reader
        .read_to_end(&mut bytes)
        .map_err(FileError::ReadError)?;
    Ok(bytes)
}

pub fn read_file<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, FileError> {
    let path = path.as_ref();
    let mut file = open_file(path)?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes)
        .map_err(|e| FileError::FileRead(path.to_path_buf(), e))?;
    Ok(bytes)
}

pub fn read_file_to_string<P: AsRef<Path>>(path: P) -> Result<String, FileError> {
    let path = path.as_ref();
    let s = fs::read_to_string(path).map_err(|e| FileError::FileRead(path.to_path_buf(), e))?;
    Ok(s)
}

pub fn read_file_to_string_lossy<P: AsRef<Path>>(path: P) -> Result<String, FileError> {
    let path = path.as_ref();
    let bytes = read_file(path)?;
    let s = String::from_utf8_lossy(&bytes).into_owned();
    Ok(s)
}

/// Note: creates all intermediary directories if needed.
pub fn create_file<P: AsRef<Path>>(path: P) -> Result<File, FileError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            create_dir_all(parent)?;
        }
    }
    File::create(path).map_err(|e| FileError::FileCreate(path.to_path_buf(), e))
}

/// Removes whatever is at the path, whether it is a directory or file. The _all suffix hopefully makes the function sound at least slightly dangerous.
pub fn remove_all<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    let path = path.as_ref();
    if path.is_file() {
        remove_file(path)
    } else if path.is_dir() {
        remove_dir_all(path)
    } else {
        Ok(())
    }
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|e| FileError::FileRemove(path.to_path_buf(), e))
}

pub fn remove_file_locked<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    let path = path.as_ref();
    let _lock = Lock::file(path, LockOptions::Write)?;
    fs::remove_file(path).map_err(|e| FileError::FileRemove(path.to_path_buf(), e))
}

pub fn write_to_file<S: AsRef<[u8]>, P: AsRef<Path>>(
    source: S,
    target: P,
) -> Result<File, FileError> {
    let target = target.as_ref();
    let mut target_file = create_file(target)?;
    target_file
        .write_all(source.as_ref())
        .map_err(|e| FileError::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
}

pub fn write_to_writer<S: AsRef<[u8]>, W: Write>(
    source: S,
    mut target: W,
) -> Result<(), FileError> {
    target
        .write_all(source.as_ref())
        .map_err(FileError::WriteError)?;
    Ok(())
}

/// Reads all of the data from source and writes it into a new file at target.
pub fn read_to_file<R: Read, P: AsRef<Path>>(source: &mut R, target: P) -> Result<File, FileError> {
    let target = target.as_ref();
    let mut target_file = create_file(target)?;
    std::io::copy(source, &mut target_file)
        .map_err(|e| FileError::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
}

pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<ReadDir, FileError> {
    fs::read_dir(&path).map_err(|e| FileError::DirRead(path.as_ref().to_path_buf(), e))
}

pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    fs::create_dir(&path).map_err(|e| FileError::DirCreate(path.as_ref().to_path_buf(), e))
}

pub fn create_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    fs::create_dir_all(&path).map_err(|e| FileError::DirCreate(path.as_ref().to_path_buf(), e))
}

pub fn remove_dir_empty<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    fs::remove_dir(&path).map_err(|e| FileError::DirRemove(path.as_ref().to_path_buf(), e))
}

pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FileError> {
    fs::remove_dir_all(&path).map_err(|e| FileError::DirRemove(path.as_ref().to_path_buf(), e))
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<(), FileError> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::rename(from, to).map_err(|e| FileError::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source: e,
    })
}

/// Copies the file or directory at source into the target path.
/// If the source is a file and the target is not a directory, the source file is copied to the target path.
/// If the source is a file and the target is a directory, the source file is copied into the target directory.
/// If the source is a directory and the target is not a file, the source directory and all files in it are copied recursively into the target directory. For example, with source=dir1 and target=dir2, dir1/file would be copied to dir2/dir1/file.
/// If the source is a directory and the target is a file, an error is returned.
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(source: P, target: Q) -> Result<(), FileError> {
    let source = source.as_ref();
    let target = target.as_ref();

    if source.is_file() {
        if target.is_dir() {
            log::trace!(
                "copying into dir {} -> {}",
                source.display(),
                target.display()
            );
            let file_name = if let Some(file_name) = source.file_name() {
                file_name
            } else {
                return Err(FileError::NoFileName(source.to_path_buf()));
            };
            let path_in_target = target.join(file_name);
            std::fs::copy(source, path_in_target).map_err(|e| FileError::FileCopy {
                from: source.to_path_buf(),
                to: target.to_path_buf(),
                source: e,
            })?;
        } else {
            log::trace!("copying file {} -> {}", source.display(), target.display());
            if let Some(parent) = target.parent() {
                if !parent.exists() {
                    create_dir_all(parent)?;
                }
            }
            std::fs::copy(source, target).map_err(|e| FileError::FileCopy {
                from: source.to_path_buf(),
                to: target.to_path_buf(),
                source: e,
            })?;
        }
    } else {
        log::trace!(
            "recursively copying {} -> {}",
            source.display(),
            target.display()
        );
        if target.is_file() {
            return Err(FileError::UnexpectedFile(target.to_path_buf()));
        } else {
            let prefix = source.parent().unwrap_or_else(|| Path::new(""));
            for entry in WalkDir::new(source) {
                let entry = entry?;
                let entry_path = entry.path();
                let stripped = entry_path
                    .strip_prefix(prefix)
                    .expect("prefix is derived from the source which entry_path is in");

                let target = target.join(stripped);
                if entry_path.is_dir() {
                    create_dir_all(target)?;
                } else {
                    if let Some(parent) = target.parent() {
                        create_dir_all(parent)?;
                    }
                    std::fs::copy(entry_path, &target).map_err(|e| FileError::FileCopy {
                        from: entry_path.to_path_buf(),
                        to: target.clone(),
                        source: e,
                    })?;
                }
            }
        }
    }
    Ok(())
}

pub fn canonicalize(path: &Path) -> Result<PathBuf, FileError> {
    let canon =
        dunce::canonicalize(path).map_err(|e| FileError::Canonicalize(path.to_path_buf(), e))?;
    Ok(canon)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::path::PathBuf;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    #[test]
    fn a_refused_lock_is_warned_about_once_and_recorded() {
        init();

        // ENOLCK (37 on Linux) is what `fcntl(F_SETLK)` answers with on an NFS
        // mount whose lock daemon is unreachable -- the case that makes locking
        // fail open, and the reason `locking_unavailable` exists.
        let enolck = std::io::Error::from_raw_os_error(37);
        let path = Path::new("/nfs/home/user/.config/tmc-test/credentials_mooc_lab.json");

        // The flag is process-wide and sticky by design; nothing else in this test
        // binary observes it, so setting it here is safe regardless of test order.
        assert!(
            report_locking_unavailable(path, &enolck),
            "the first degradation must be reported"
        );
        assert!(
            !report_locking_unavailable(path, &enolck),
            "later degradations must not repeat the warning"
        );
        assert!(
            locking_unavailable(),
            "the degradation must stay visible to callers that need a real lock"
        );
    }

    /// Set on the child process spawned by
    /// [`with_file_lock_timeout_gives_up_when_another_process_holds_the_lock`] to
    /// tell it which file to lock.
    const HOLD_LOCK_VAR: &str = "TMC_TEST_HOLD_LOCK_PATH";

    /// Not a test in its own right: this is the entry point of the child process
    /// the contention test spawns, and it does nothing in an ordinary test run.
    ///
    /// A separate process is unavoidable — the unix locks are `fcntl` locks, which
    /// are per-process, so a second `Lock` on the same file inside the test process
    /// would never contend. The child takes the lock, signals that it holds it, and
    /// then parks until the parent kills it, so the parent's assertion doesn't race
    /// with the child's lifetime.
    #[test]
    fn child_process_holds_a_lock() {
        let Ok(path) = std::env::var(HOLD_LOCK_VAR) else {
            return;
        };
        let path = PathBuf::from(path);
        let mut lock = Lock::file(&path, LockOptions::WriteCreate).unwrap();
        let _guard = lock.lock().unwrap();
        std::fs::write(path.with_extension("held"), b"1").unwrap();
        std::thread::sleep(Duration::from_secs(120));
    }

    #[test]
    fn with_file_lock_timeout_gives_up_when_another_process_holds_the_lock() {
        init();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked");
        let held_marker = path.with_extension("held");

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["file_util::test::child_process_holds_a_lock", "--exact"])
            .env(HOLD_LOCK_VAR, &path)
            .spawn()
            .unwrap();

        // Wait for the child to actually hold the lock before asserting anything.
        // The bound here only guards against a child that never starts; it is not
        // part of what the test asserts.
        let waiting_since = Instant::now();
        while !held_marker.exists() {
            assert!(
                waiting_since.elapsed() < Duration::from_secs(60),
                "the child process never took the lock"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // The child holds the lock until we kill it, so this must time out rather
        // than block forever.
        let result = with_file_lock_timeout(
            &path,
            LockOptions::WriteCreate,
            Duration::from_millis(200),
            |_guard| (),
        );
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            matches!(result, Err(FileError::LockTimeout { .. })),
            "expected a lock timeout, got {result:?}"
        );

        // With the holder gone the same call succeeds, so the timeout above was
        // contention and not a broken code path.
        with_file_lock_timeout(
            &path,
            LockOptions::WriteCreate,
            Duration::from_millis(200),
            |_guard| (),
        )
        .unwrap();
    }

    #[test]
    fn with_file_lock_timeout_runs_the_closure_and_returns_its_value() {
        init();

        let dir = tempfile::tempdir().unwrap();
        // A direct child of an existing directory: only the unix `Lock::file`
        // creates missing parents, so a nested path wouldn't behave the same on
        // every platform.
        let path = dir.path().join("file");
        let value = with_file_lock_timeout(
            &path,
            LockOptions::WriteCreate,
            Duration::from_secs(1),
            |guard| {
                // The guard exposes the locked file itself.
                guard.get_file().metadata().unwrap().is_file()
            },
        )
        .unwrap();
        assert!(value);
        assert!(path.exists(), "WriteCreate should have created the file");
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    fn dir_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        std::fs::create_dir_all(&target).unwrap();
        target
    }

    #[test]
    fn copies_file_to_file() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/file", "file contents");

        let target = tempfile::tempdir().unwrap();
        copy(
            temp.path().join("dir/file"),
            target.path().join("another/place"),
        )
        .unwrap();

        let conts = read_file_to_string(target.path().join("another/place")).unwrap();
        assert_eq!(conts, "file contents");
    }

    #[test]
    fn copies_file_to_dir() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/file", "file contents");

        let target = tempfile::tempdir().unwrap();
        dir_to(&target, "some/dir");
        copy(temp.path().join("dir/file"), target.path().join("some/dir")).unwrap();

        let conts = read_file_to_string(target.path().join("some/dir/file")).unwrap();
        assert_eq!(conts, "file contents");
    }

    #[test]
    fn copies_dir() {
        init();
        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/another/file", "file contents");
        file_to(&temp, "dir/elsewhere/f", "another file");
        dir_to(&temp, "dir/some dir");

        let target = tempfile::tempdir().unwrap();
        copy(temp.path().join("dir"), target.path()).unwrap();

        let conts = read_file_to_string(target.path().join("dir/another/file")).unwrap();
        assert_eq!(conts, "file contents");
        let conts = read_file_to_string(target.path().join("dir/elsewhere/f")).unwrap();
        assert_eq!(conts, "another file");
        assert!(target.path().join("dir/some dir").is_dir());
    }
}
