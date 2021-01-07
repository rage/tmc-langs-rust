//! Various utility functions, primarily wrapping the standard library's IO and filesystem functions

use crate::error::FileIo;
use fd_lock::{FdLock, FdLockGuard};
use std::io::{Read, Write};
use std::path::Path;
use std::{
    fs::{self, File},
    path::PathBuf,
};
use walkdir::WalkDir;

pub fn temp_file() -> Result<File, FileIo> {
    tempfile::tempfile().map_err(FileIo::TempFile)
}

pub fn open_file<P: AsRef<Path>>(path: P) -> Result<File, FileIo> {
    let path = path.as_ref();
    File::open(path).map_err(|e| FileIo::FileOpen(path.to_path_buf(), e))
}

pub fn read_file<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, FileIo> {
    let path = path.as_ref();
    let mut file = open_file(path)?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes)
        .map_err(|e| FileIo::FileRead(path.to_path_buf(), e))?;
    Ok(bytes)
}

pub fn read_file_to_string<P: AsRef<Path>>(path: P) -> Result<String, FileIo> {
    let path = path.as_ref();
    let s = fs::read_to_string(path).map_err(|e| FileIo::FileRead(path.to_path_buf(), e))?;
    Ok(s)
}

pub fn create_file<P: AsRef<Path>>(path: P) -> Result<File, FileIo> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            create_dir_all(parent)?;
        }
    }
    File::create(path).map_err(|e| FileIo::FileCreate(path.to_path_buf(), e))
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|e| FileIo::FileRemove(path.to_path_buf(), e))
}

pub fn write_to_file<S: AsRef<[u8]>, P: AsRef<Path>>(source: S, target: P) -> Result<File, FileIo> {
    let target = target.as_ref();
    let mut target_file = create_file(target)?;
    target_file
        .write_all(source.as_ref())
        .map_err(|e| FileIo::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
}

/// Reads all of the data from source and writes it into a new file at target.
pub fn read_to_file<R: Read, P: AsRef<Path>>(source: &mut R, target: P) -> Result<File, FileIo> {
    let target = target.as_ref();
    let mut target_file = create_file(target)?;
    std::io::copy(source, &mut target_file)
        .map_err(|e| FileIo::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
}

pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    fs::create_dir(&path).map_err(|e| FileIo::DirCreate(path.as_ref().to_path_buf(), e))
}

pub fn create_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    fs::create_dir_all(&path).map_err(|e| FileIo::DirCreate(path.as_ref().to_path_buf(), e))
}

pub fn remove_dir_empty<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    fs::remove_dir(&path).map_err(|e| FileIo::DirRemove(path.as_ref().to_path_buf(), e))
}

pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    fs::remove_dir_all(&path).map_err(|e| FileIo::DirRemove(path.as_ref().to_path_buf(), e))
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<(), FileIo> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::rename(from, to).map_err(|e| FileIo::Rename {
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
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(source: P, target: Q) -> Result<(), FileIo> {
    let source = source.as_ref();
    let target = target.as_ref();

    if source.is_file() {
        if target.is_dir() {
            log::debug!(
                "copying into dir {} -> {}",
                source.display(),
                target.display()
            );
            let file_name = if let Some(file_name) = source.file_name() {
                file_name
            } else {
                return Err(FileIo::NoFileName(source.to_path_buf()));
            };
            let path_in_target = target.join(file_name);
            std::fs::copy(source, path_in_target).map_err(|e| FileIo::FileCopy {
                from: source.to_path_buf(),
                to: target.to_path_buf(),
                source: e,
            })?;
        } else {
            log::debug!("copying file {} -> {}", source.display(), target.display());
            if let Some(parent) = target.parent() {
                if !parent.exists() {
                    create_dir_all(parent)?;
                }
            }
            std::fs::copy(source, target).map_err(|e| FileIo::FileCopy {
                from: source.to_path_buf(),
                to: target.to_path_buf(),
                source: e,
            })?;
        }
    } else {
        log::debug!(
            "recursively copying {} -> {}",
            source.display(),
            target.display()
        );
        if target.is_file() {
            return Err(FileIo::UnexpectedFile(target.to_path_buf()));
        } else {
            let prefix = source.parent().unwrap_or_else(|| Path::new(""));
            for entry in WalkDir::new(source) {
                let entry = entry?;
                let entry_path = entry.path();
                let stripped = entry_path.strip_prefix(prefix).unwrap();

                let target = target.join(stripped);
                if entry_path.is_dir() {
                    create_dir_all(target)?;
                } else {
                    if let Some(parent) = target.parent() {
                        create_dir_all(parent)?;
                    }
                    std::fs::copy(entry_path, &target).map_err(|e| FileIo::FileCopy {
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

#[macro_export]
macro_rules! lock {
    ( $( $path: expr ),+ ) => {
        $(
            let mut fl = $crate::file_util::FileLock::new(&$path)?;
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
    pub fn new(path: impl Into<PathBuf>) -> Result<FileLock, FileIo> {
        let path = path.into();
        let file = open_file(&path)?;
        Ok(Self {
            path,
            fd_lock: FdLock::new(file),
        })
    }

    /// Blocks until the lock can be acquired.
    pub fn lock(&mut self) -> Result<FdLockGuard<'_, File>, FileIo> {
        let path = &self.path;
        let fd_lock = &mut self.fd_lock;
        Ok(fd_lock
            .lock()
            .map_err(|e| FileIo::FdLock(path.clone(), e))?)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::PathBuf;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
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
