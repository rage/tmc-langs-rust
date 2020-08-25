//! Various utility functions, primarily wrapping the standard library's IO and filesystem functions

use crate::error::{FileIo, TmcError};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::{read::ZipFile, ZipArchive};

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
        create_dir_all(parent)?;
    }
    File::create(path).map_err(|e| FileIo::FileCreate(path.to_path_buf(), e))
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), FileIo> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|e| FileIo::FileRemove(path.to_path_buf(), e))
}

pub fn write_to_file<S: AsRef<[u8]>, P: AsRef<Path>>(source: S, target: P) -> Result<File, FileIo> {
    let target = target.as_ref();
    if let Some(parent) = target.parent() {
        create_dir_all(parent)?;
    }
    let mut target_file = create_file(target)?;
    target_file
        .write_all(source.as_ref())
        .map_err(|e| FileIo::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
}

pub fn read_to_file<R: Read, P: AsRef<Path>>(source: &mut R, target: P) -> Result<File, FileIo> {
    let target = target.as_ref();
    if let Some(parent) = target.parent() {
        create_dir_all(parent)?;
    }
    let mut target_file = create_file(target)?;
    std::io::copy(source, &mut target_file)
        .map_err(|e| FileIo::FileWrite(target.to_path_buf(), e))?;
    Ok(target_file)
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

pub fn find_project_root<P: AsRef<Path>>(path: P) -> Result<Option<PathBuf>, FileIo> {
    for entry in WalkDir::new(path) {
        let entry = entry?;
        if entry.path().is_dir() && entry.file_name() == OsStr::new("src") {
            return Ok(entry.path().parent().map(Path::to_path_buf));
        }
    }
    Ok(None)
}

/// Copies the file or directory at source into the target path.
/// If the source is a file and the target is not a directory, the source file is copied to the target path.
/// If the source is a file and the target is a directory, the source file is copied into the target directory.
/// If the source is a directory and the target is not a file, the source directory and all files in it are copied recursively into the target directory. For example, with source=dir1 and target=dir2, dir1/file would be copied to dir2/dir1/file.
/// If the source is a directory and the target is a file, an error is returned.
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(source: P, target: Q) -> Result<(), FileIo> {
    let source = source.as_ref();
    let target = target.as_ref();
    log::debug!("copying {} -> {}", source.display(), target.display());

    if source.is_file() {
        if target.is_dir() {
            // copy source into target dir
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
            // copy source into target path
            std::fs::copy(source, target).map_err(|e| FileIo::FileCopy {
                from: source.to_path_buf(),
                to: target.to_path_buf(),
                source: e,
            })?;
        }
    } else {
        // recursively copy contents of source to target
        if target.is_file() {
            return Err(FileIo::UnexpectedFile(target.to_path_buf()));
        } else {
            let prefix = source.parent().unwrap_or_else(|| Path::new(""));
            for entry in WalkDir::new(source) {
                let entry = entry?;
                let entry_path = entry.path();
                debug_assert!(dbg!(entry_path).exists());
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

pub fn unzip<P: AsRef<Path>, Q: AsRef<Path>, F>(
    zip_path: P,
    target: Q,
    filter: F,
) -> Result<(), TmcError>
where
    F: Fn(&ZipFile) -> bool,
{
    let zip_path = zip_path.as_ref();

    let target = target.as_ref();
    log::debug!("unzip from {} to {}", zip_path.display(), target.display());

    let archive = open_file(zip_path)?;
    let mut archive = ZipArchive::new(archive)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if filter(&file) {
            continue;
        }

        let target_path = target.join(file.sanitized_name());
        if file.is_dir() {
            create_dir_all(target_path)?;
        } else {
            read_to_file(&mut file, target_path)?;
        }
    }
    Ok(())
}
