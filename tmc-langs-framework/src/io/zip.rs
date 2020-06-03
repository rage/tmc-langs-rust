//! Contains functions for zipping and unzipping projects.

use crate::policy::StudentFilePolicy;
use crate::{Error, Result};
use log::debug;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use std::path::PathBuf;
use walkdir::{DirEntry, WalkDir};
pub use zip::result::ZipError;
use zip::{write::FileOptions, ZipArchive, ZipWriter};

/// Zips the given directory, only including student files according to the given policy.
pub fn zip(policy: Box<dyn StudentFilePolicy>, root_directory: &Path) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(vec![]));
    let tmc_project_yml = policy.get_tmc_project_yml()?;

    for entry in WalkDir::new(root_directory)
        .into_iter()
        .filter_entry(|e| !contains_tmcnosubmit(e))
        .filter_map(|e| e.ok())
    {
        debug!("processing {:?}", entry.path());
        if policy.is_student_file(entry.path(), &root_directory, &tmc_project_yml)? {
            let path = root_directory
                .parent()
                .map(|p| entry.path().strip_prefix(p).unwrap())
                .unwrap_or_else(|| entry.path());
            if entry.path().is_dir() {
                debug!("adding directory {}", path.display());
                writer.add_directory_from_path(path, FileOptions::default())?;
            } else {
                let file = File::open(entry.path())?;
                let bytes = file
                    .bytes()
                    .collect::<std::result::Result<Vec<_>, std::io::Error>>()?;
                debug!("writing file {}", path.display());
                writer.start_file_from_path(path, FileOptions::default())?;
                writer
                    .write_all(&bytes)
                    .map_err(|e| Error::Write(path.to_path_buf(), e))?;
            }
        }
    }
    let cursor = writer.finish()?;
    Ok(cursor.into_inner())
}

/// Finds a project directory in the given zip and unzips it.
pub fn unzip(policy: Box<dyn StudentFilePolicy>, zip: &Path, target: &Path) -> Result<()> {
    debug!("Unzipping {} to {}", zip.display(), target.display());

    let file = File::open(zip).map_err(|e| Error::OpenFile(zip.to_path_buf(), e))?;
    let mut zip_archive = ZipArchive::new(file)?;

    let project_dir = find_project_dir(&mut zip_archive)?;
    debug!("Project dir in zip: {}", project_dir.display());

    let tmc_project_yml = policy.get_tmc_project_yml()?;

    let mut unzipped_paths = HashSet::new();

    for i in 0..zip_archive.len() {
        let file = zip_archive.by_index(i)?;
        let file_path = file.sanitized_name();
        if !file_path.starts_with(&project_dir) {
            debug!("skip {}, not in project dir", file.name());
            continue;
        }
        let relative = file_path.strip_prefix(&project_dir).unwrap();
        let path_in_target = target.join(&relative);
        debug!("processing {:?} -> {:?}", file_path, path_in_target);

        if file.is_dir() {
            debug!("creating {:?}", path_in_target);
            fs::create_dir_all(&path_in_target)
                .map_err(|e| Error::CreateDir(path_in_target.clone(), e))?;
            unzipped_paths.insert(path_in_target.canonicalize()?);
        } else {
            let mut write = true;
            let file_contents = file.bytes().collect::<std::result::Result<Vec<_>, _>>()?;
            // always overwrite .tmcproject.yml
            if path_in_target.exists()
                && !path_in_target
                    .file_name()
                    .map(|o| o == ".tmcproject.yml")
                    .unwrap_or_default()
            {
                let target_file = File::open(&path_in_target)
                    .map_err(|e| Error::OpenFile(path_in_target.clone(), e))?;
                let target_file_contents = target_file
                    .bytes()
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if file_contents == target_file_contents
                    || (policy.is_student_file(&path_in_target, &target, &tmc_project_yml)?
                        && !policy.is_updating_forced(&path_in_target, &tmc_project_yml)?)
                {
                    write = false;
                }
            }
            if write {
                debug!("writing to {}", path_in_target.display());
                if let Some(res) = path_in_target.parent().map(fs::create_dir_all) {
                    res?;
                }
                let mut overwrite_target = File::create(&path_in_target)
                    .map_err(|e| Error::CreateFile(path_in_target.clone(), e))?;
                overwrite_target
                    .write_all(&file_contents)
                    .map_err(|e| Error::Write(path_in_target.clone(), e))?;
                unzipped_paths.insert(path_in_target.canonicalize()?);
            }
        }
    }

    // delete non-student files that were not in zip
    debug!("deleting non-student files not in zip");
    for entry in WalkDir::new(target).into_iter().filter_map(|e| e.ok()) {
        if !unzipped_paths.contains(&entry.path().canonicalize()?)
            && (policy.is_updating_forced(entry.path(), &tmc_project_yml)?
                || !policy.is_student_file(entry.path(), &project_dir, &tmc_project_yml)?)
        {
            if entry.path().is_dir() {
                // delete if empty
                if WalkDir::new(entry.path()).max_depth(1).into_iter().count() == 1 {
                    debug!("deleting empty directory {}", entry.path().display());
                    fs::remove_dir(entry.path())?;
                }
            } else {
                debug!("removing file {}", entry.path().display());
                fs::remove_file(entry.path())?;
            }
        }
    }

    Ok(())
}

fn find_project_dir<R: Read + Seek>(zip_archive: &mut ZipArchive<R>) -> Result<PathBuf> {
    for i in 0..zip_archive.len() {
        let file = zip_archive.by_index(i)?;
        let file_path = file.sanitized_name();
        let file_name = file_path.file_name().unwrap_or_default();
        if file.is_dir() && (file_name == "nbproject" || file_name == "src" || file_name == "test")
            || file.is_file()
                && (file_name == "pom.xml" || file_name == ".idea" || file_name == "Makefile")
        {
            let parent = file_path.parent().unwrap_or_else(|| Path::new(""));
            debug!("found project dir {}", parent.display());
            return Ok(parent.to_path_buf());
        }
    }
    Err(Error::NoProjectDirInZip)
}

fn contains_tmcnosubmit(entry: &DirEntry) -> bool {
    for entry in WalkDir::new(entry.path())
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_name() == ".tmcnosubmit" {
            debug!("contains .tmcnosubmit: {}", entry.path().display());
            return true;
        }
    }
    false
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::policy::EverythingIsStudentFilePolicy;
    use std::collections::HashSet;
    use tempfile::tempdir;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn get_relative_file_paths(dir: &Path) -> HashSet<PathBuf> {
        WalkDir::new(dir)
            .into_iter()
            .map(|e| e.unwrap())
            .map(|e| e.into_path())
            .filter(|e| e.is_file())
            .map(|e| e.strip_prefix(dir).unwrap().to_path_buf())
            .collect()
    }

    #[test]
    fn zips() {
        init();

        let temp = tempdir().unwrap();
        let student_file_path = temp
            .path()
            .join("exercise-name/src/main/java/AdaLovelace.java");
        let missing_file_path = temp.path().join("exercise-name/pom.xml");
        fs::create_dir_all(student_file_path.parent().unwrap()).unwrap();
        File::create(student_file_path).unwrap();
        fs::create_dir_all(missing_file_path.parent().unwrap()).unwrap();
        File::create(missing_file_path).unwrap();

        let zipped = zip(
            Box::new(EverythingIsStudentFilePolicy {}),
            &temp.path().join("exercise-name"),
        )
        .unwrap();
        let mut archive = ZipArchive::new(Cursor::new(zipped)).unwrap();
        for i in 0..archive.len() {
            println!("{:?}", archive.by_index(i).unwrap().name());
        }
        assert!(archive
            .by_name("exercise-name/src/main/java/AdaLovelace.java")
            .is_ok());
        assert!(archive.by_name("exercise-name/pom.xml").is_ok());
    }

    #[test]
    fn unzipping_nonexisting_errors() {
        init();

        assert!(unzip(
            Box::new(EverythingIsStudentFilePolicy {}),
            Path::new("nonexistent"),
            Path::new(""),
        )
        .is_err())
    }

    #[test]
    fn unzips_simple() {
        init();

        let temp = tempdir().unwrap();
        unzip(
            Box::new(EverythingIsStudentFilePolicy {}),
            Path::new("tests/data/zip/module-trivial.zip"),
            temp.path(),
        )
        .unwrap();

        let expected = get_relative_file_paths(Path::new("tests/data/zip/module-trivial"));
        let actual = get_relative_file_paths(temp.path());
        assert_eq!(expected, actual)
    }

    #[test]
    fn unzips_complex() {
        init();

        let temp = tempdir().unwrap();
        unzip(
            Box::new(EverythingIsStudentFilePolicy {}),
            Path::new("tests/data/zip/course-module-trivial.zip"),
            temp.path(),
        )
        .unwrap();

        let expected = get_relative_file_paths(Path::new("tests/data/zip/module-trivial"));
        let actual = get_relative_file_paths(temp.path());
        assert_eq!(expected, actual)
    }
}
