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
    fs::{self, File, OpenOptions, ReadDir},
    io::{Read, Write},
    path::{Path, PathBuf},
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
            Self::ReadCreate => opts.read(true).create(true),
            // truncate requires write
            Self::ReadTruncate => opts.write(true).truncate(true),
            Self::Write => opts.write(true),
            Self::WriteCreate => opts.write(true).create(true),
            Self::WriteTruncate => opts.write(true).truncate(true),
        };
        opts
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
