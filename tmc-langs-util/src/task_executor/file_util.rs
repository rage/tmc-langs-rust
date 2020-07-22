//! Various utility functions, primarily wrapping the standard library's IO and filesystem functions

use crate::TmcError;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::{read::ZipFile, ZipArchive};

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
            create_dir(target_path)?;
        } else {
            write_to_file(&mut file, target_path)?;
        }
    }
    Ok(())
}

pub fn create_file<P: AsRef<Path>>(path: P) -> Result<File, TmcError> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.exists() {
            create_dir(parent)?;
        }
    }
    File::create(&path).map_err(|e| TmcError::CreateFile(path.as_ref().to_path_buf(), e))
}

pub fn open_file<P: AsRef<Path>>(path: P) -> Result<File, TmcError> {
    File::open(&path).map_err(|e| TmcError::FileOpen(path.as_ref().to_path_buf(), e))
}

pub fn write_to_file<R: Read, P: AsRef<Path>>(source: &mut R, target: P) -> Result<File, TmcError> {
    let target = target.as_ref();
    if let Some(parent) = target.parent() {
        create_dir(parent)?;
    }
    let mut target_file = create_file(target)?;
    std::io::copy(source, &mut target_file)
        .map_err(|e| TmcError::Write(target.to_path_buf(), e))?;
    Ok(target_file)
}

pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<(), TmcError> {
    fs::create_dir_all(&path).map_err(|e| TmcError::CreateDir(path.as_ref().to_path_buf(), e))
}

pub fn find_project_root<P: AsRef<Path>>(path: P) -> Result<Option<PathBuf>, TmcError> {
    for entry in WalkDir::new(path) {
        let entry = entry?;
        if entry.path().is_dir() && entry.file_name() == OsStr::new("src") {
            return Ok(entry.path().parent().map(Path::to_path_buf));
        }
    }
    Ok(None)
}

pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(source: P, target: Q) -> Result<(), TmcError> {
    let source = source.as_ref();
    let target = target.as_ref();
    log::debug!("copying {} -> {}", source.display(), target.display());

    let prefix = source.parent().unwrap_or_else(|| Path::new(""));

    for entry in WalkDir::new(source) {
        let entry = entry?;
        let entry_path = entry.path();
        let stripped = entry_path.strip_prefix(prefix).unwrap();

        let target = target.join(stripped);
        if entry_path.is_dir() {
            fs::create_dir_all(target)
                .map_err(|e| TmcError::CreateDir(entry.path().to_path_buf(), e))?;
        } else {
            log::debug!(
                "copying file {} -> {}",
                entry_path.display(),
                target.display()
            );
            std::fs::copy(&entry_path, &target).map_err(|e| {
                TmcError::FileCopy(entry_path.to_path_buf(), target.to_path_buf(), e)
            })?;
        }
    }
    Ok(())
}
