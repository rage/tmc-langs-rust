//! Contains functions for compressing and uncompressing projects.

use blake3::Hash;
use std::{
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
};
use tmc_langs_framework::{
    ArchiveBuilder, Compression, Entry, EntryNameStyle, ExtractionBudget, ExtractionLimits,
    StudentFilePolicy, TmcError, validate_entry_name,
};
use tmc_langs_util::file_util;
use walkdir::{DirEntry, WalkDir};
use zip::ZipArchive;
pub use zip::result::ZipError;

/// Compresses the given directory, only including student files according to the given policy.
pub fn compress_student_files(
    policy: &dyn StudentFilePolicy,
    root_directory: &Path,
    compression: Compression,
    deterministic: bool,
    hash: bool,
    size_limit_mb: u32,
) -> Result<(Vec<u8>, Option<Hash>), TmcError> {
    let mut writer = ArchiveBuilder::new(
        Cursor::new(vec![]),
        compression,
        Some(size_limit_mb),
        deterministic,
        hash,
    );
    for entry in WalkDir::new(root_directory)
        .sort_by(|a, b| a.path().cmp(b.path()))
        .into_iter()
        .filter_entry(|e| !contains_tmcnosubmit(e))
        .filter_map(|e| {
            e.inspect_err(|e| log::warn!("leaving unreadable entry out of the archive: {e}"))
                .ok()
        })
    {
        let relative = entry
            .path()
            .strip_prefix(root_directory)
            .expect("all entries are inside root");
        log::trace!(
            "processing {} ({})",
            entry.path().display(),
            relative.display()
        );
        if entry.file_type().is_symlink() {
            log::warn!("not packing symbolic link {}", entry.path().display());
            continue;
        }
        if entry.file_type().is_file() && is_sync_conflict_copy(entry.path()) {
            log::warn!("not packing sync conflict copy {}", entry.path().display());
            continue;
        }
        if policy.is_student_file(relative) {
            let path = root_directory
                .parent()
                .map(|p| {
                    entry
                        .path()
                        .strip_prefix(p)
                        .expect("entries are inside root_directory")
                })
                .unwrap_or_else(|| entry.path());
            if entry.file_type().is_dir() {
                writer.add_directory(entry.path(), path)?;
            } else {
                writer.add_file(entry.path(), path)?;
            }
        }
    }
    let (cursor, hash) = writer.finish()?;
    let size_limit_b = usize::try_from(size_limit_mb)
        .unwrap_or(usize::MAX)
        .saturating_mul(1000 * 1000);
    if cursor.get_ref().len() > size_limit_b {
        return Err(TmcError::ArchiveSizeLimitExceeded {
            limit: size_limit_mb,
        });
    }
    let data = cursor.into_inner();
    Ok((data, hash))
}

/// Finds a project directory in the given zip and unzips it according to the given student policy. Also cleans unnecessary non-student files.
pub fn unzip(zip: impl std::io::Read + std::io::Seek, target: &Path) -> Result<(), TmcError> {
    log::debug!("Unzipping to {}", target.display());

    let mut zip_archive = ZipArchive::new(zip)?;

    let project_dir = find_project_dir(&mut zip_archive)?;
    log::debug!("Project dir in zip: {}", project_dir.display());

    let mut budget = ExtractionBudget::new(ExtractionLimits::default());
    for i in 0..zip_archive.len() {
        let mut file = Entry::Zip(zip_archive.by_index(i)?);
        let file_path = validate_entry_name(&file.raw_name(), EntryNameStyle::Zip)?;
        budget.admit_entry(&file.raw_name(), file.size())?;
        let relative = match file_path.strip_prefix(&project_dir) {
            Ok(relative) => relative,
            _ => {
                log::trace!("skip {}, not in project dir", file.raw_name());
                continue;
            }
        };
        let path_in_target = target.join(relative);
        log::trace!("processing {file_path:?} -> {path_in_target:?}");

        if file.is_dir() {
            log::trace!("creating {path_in_target:?}");
            file_util::create_dir_all(&path_in_target)?;
        } else {
            log::trace!("writing to {}", path_in_target.display());
            file.write_to_file(&path_in_target, &mut budget)?;
        }
    }

    Ok(())
}

// TODO: make more robust, use language plugins?
fn find_project_dir<R: Read + Seek>(zip_archive: &mut ZipArchive<R>) -> Result<PathBuf, TmcError> {
    let mut lowest_ipynb_dir = None::<PathBuf>;

    for i in 0..zip_archive.len() {
        let file = zip_archive.by_index(i)?;
        let file_path = Path::new(file.name());

        // directories may not have entries in the zip, e.g. it may only have
        // exercise/src/main... without an entry for src, so we need to check
        // the path components to find src
        let mut components = file_path.components().peekable();
        let mut collected = vec![];
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                // do not inspect the last component,
                // they will have an entry that is
                // processed in the next step
                break;
            }

            let comp = component.as_os_str();
            if comp == "nbproject" || comp == "src" || comp == "test" {
                let path: PathBuf = collected.into_iter().collect();
                return Ok(path);
            }
            collected.push(comp);
        }

        let file_name = file_path.file_name().unwrap_or_default();
        if file.is_dir() && (file_name == "nbproject" || file_name == "src" || file_name == "test")
            || file.is_file()
                && (file_name == "pom.xml" || file_name == ".idea" || file_name == "Makefile")
        {
            let parent = file_path.parent().unwrap_or_else(|| Path::new(""));
            log::debug!("found project dir {}", parent.display());
            return Ok(parent.to_path_buf());
        }

        if file_path
            .extension()
            .map(|ext| ext == "ipynb")
            .unwrap_or_default()
        {
            let parent = file_path.parent().unwrap_or_else(|| Path::new(""));
            if let Some(lowest_ipynb_dir) = lowest_ipynb_dir.as_mut() {
                if lowest_ipynb_dir.components().count() > parent.components().count() {
                    *lowest_ipynb_dir = parent.to_path_buf();
                }
            } else {
                lowest_ipynb_dir = Some(parent.to_path_buf());
            }
        }
    }
    if let Some(lowest_ipynb_dir) = lowest_ipynb_dir {
        Ok(lowest_ipynb_dir)
    } else {
        Err(TmcError::NoProjectDirInArchive)
    }
}

/// Whether the file looks like a copy that a file-sync client or an office program made next to a
/// student file, which must not be submitted. Conservative: ambiguous OneDrive-style names count
/// only when the file they would be a copy of exists beside them.
fn is_sync_conflict_copy(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if is_unambiguous_conflict_artifact(name) {
        return true;
    }
    onedrive_original_candidates(name)
        .iter()
        .any(|original| path.with_file_name(original).is_file())
}

fn is_unambiguous_conflict_artifact(name: &str) -> bool {
    // `~$` Office lock files, Syncthing, Dropbox: "name (Alice's conflicted copy 2024-01-31).ext"
    name.starts_with("~$") || name.contains(".sync-conflict-") || is_dropbox_conflict_copy(name)
}

fn is_dropbox_conflict_copy(name: &str) -> bool {
    const MARKER: &str = "'s conflicted copy ";
    let Some(marker_start) = name.find(MARKER) else {
        return false;
    };
    let before = &name[..marker_start];
    let after = &name[marker_start + MARKER.len()..];
    let is_date = after.get(..10).is_some_and(|d| {
        d.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
    });
    before.contains(" (") && is_date && after[10..].starts_with(')')
}

/// File names a OneDrive conflict copy `name-HOSTNAME[-n].ext` could be a copy of.
fn onedrive_original_candidates(name: &str) -> Vec<String> {
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    // Windows names machines like `DESKTOP-AB12CD3`. Requiring a digit or a dash keeps real
    // student files such as `main-OLD.c` or `input-V2.txt` out of the match.
    let is_hostname = |host: &str| {
        let last_part = host.rsplit('-').next().unwrap_or_default();
        host.len() >= 3
            && host.bytes().any(|b| b.is_ascii_uppercase())
            && host
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
            && !host.starts_with('-')
            && !last_part.is_empty()
            && !last_part.bytes().all(|b| b.is_ascii_digit())
            && host.bytes().any(|b| b.is_ascii_digit() || b == b'-')
    };
    stem.match_indices('-')
        .map(|(i, _)| i)
        .filter(|&i| i > 0)
        .filter(|&i| {
            let tail = &stem[i + 1..];
            let without_counter = match tail.rsplit_once('-') {
                Some((rest, counter))
                    if !counter.is_empty() && counter.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    rest
                }
                _ => tail,
            };
            is_hostname(tail) || is_hostname(without_counter)
        })
        .map(|i| format!("{}{extension}", &stem[..i]))
        .collect()
}

fn contains_tmcnosubmit(entry: &DirEntry) -> bool {
    for entry in WalkDir::new(entry.path())
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_name() == ".tmcnosubmit" {
            log::debug!("contains .tmcnosubmit: {}", entry.path().display());
            return true;
        }
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::{
        collections::HashSet,
        fs::{self, *},
    };
    use tempfile::tempdir;
    use tmc_langs_framework::{EverythingIsStudentFilePolicy, TmcProjectYml};

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
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

        let path = temp.path().join("exercise-name");
        let tmcprojectyml = TmcProjectYml::load_or_default(&path).unwrap();
        let (zipped, _hash) = compress_student_files(
            &EverythingIsStudentFilePolicy::new(&path).unwrap(),
            &path,
            Compression::Zip,
            true,
            false,
            tmcprojectyml.get_submission_size_limit_mb(),
        )
        .unwrap();
        let mut archive = ZipArchive::new(Cursor::new(zipped)).unwrap();
        assert!(!archive.is_empty());
        for i in 0..archive.len() {
            log::debug!("{:?}", archive.by_index(i).unwrap().name());
        }
        assert!(
            archive
                .by_name("exercise-name/src/main/java/AdaLovelace.java")
                .is_ok()
        );
        assert!(archive.by_name("exercise-name/pom.xml").is_ok());
    }

    #[test]
    fn unzips_simple() {
        init();

        let temp = tempdir().unwrap();
        let zip = file_util::open_file("tests/data/zip/module-trivial.zip").unwrap();
        unzip(zip, temp.path()).unwrap();

        let expected = get_relative_file_paths(Path::new("tests/data/zip/module-trivial"));
        let actual = get_relative_file_paths(temp.path());
        assert_eq!(expected, actual)
    }

    #[test]
    fn unzips_complex() {
        init();

        let temp = tempdir().unwrap();
        let zip = file_util::open_file("tests/data/zip/course-module-trivial.zip").unwrap();
        unzip(zip, temp.path()).unwrap();

        let expected = get_relative_file_paths(Path::new("tests/data/zip/module-trivial"));
        let actual = get_relative_file_paths(temp.path());
        assert_eq!(expected, actual)
    }

    #[test]
    fn no_src_entry() {
        init();

        let temp = tempdir().unwrap();
        let zip = file_util::open_file("tests/data/zip/no-src-entry.zip").unwrap();
        unzip(zip, temp.path()).unwrap();
        assert!(temp.path().join("src").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_get_converted() {
        let zipped = file_util::read_file("tests/data/zip/compressed.zip").unwrap();
        let mut ziparch = ZipArchive::new(Cursor::new(zipped)).unwrap();
        assert!(ziparch.len() > 0);
        for i in 0..ziparch.len() {
            let file = ziparch.by_index(i).unwrap();
            assert!(file.name().chars().find(|c| c == &'\\').is_none())
        }
    }

    #[test]
    fn conflict_artifacts_are_recognised() {
        for name in [
            "~$report.docx",
            "Main.sync-conflict-20240131-101010-ABCDEFG.java",
            "main (Alice's conflicted copy 2024-01-31).py",
            "main (Alice Smith's conflicted copy 2024-01-31) (1).py",
        ] {
            assert!(is_unambiguous_conflict_artifact(name), "{name}");
        }
        for name in [
            "Main.java",
            "main (1).py",
            "my-file.py",
            "conflicted copy.py",
            "its conflicted copy 2024-01-31.py",
            "a (Bob's conflicted copy.py",
            "notes~.txt",
            "~dollar.txt",
            "$~x.txt",
        ] {
            assert!(!is_unambiguous_conflict_artifact(name), "{name}");
        }
    }

    #[test]
    fn onedrive_candidates_need_a_machine_like_host() {
        assert_eq!(
            onedrive_original_candidates("Main-DESKTOP-AB12CD3.java"),
            ["Main.java", "Main-DESKTOP.java"]
        );
        assert_eq!(
            onedrive_original_candidates("Main-DESKTOP-AB12CD3-1.java"),
            ["Main.java", "Main-DESKTOP.java"]
        );
        assert_eq!(onedrive_original_candidates("Main-PC7.java"), ["Main.java"]);
        for name in [
            "my-file.py",
            "Main.java",
            "-PC7.java",
            "a-1.py",
            "main-OLD.c",
            "main-OLD-2.c",
            "input-V2.txt",
            "Main-LAPTOP.java",
        ] {
            assert!(onedrive_original_candidates(name).is_empty(), "{name}");
        }
    }

    #[test]
    fn onedrive_copy_is_skipped_only_beside_its_original() {
        let temp = tempdir().unwrap();
        let copy = temp.path().join("Main-DESKTOP-AB12CD3-1.java");
        File::create(&copy).unwrap();
        assert!(!is_sync_conflict_copy(&copy));
        File::create(temp.path().join("Main.java")).unwrap();
        assert!(is_sync_conflict_copy(&copy));
    }

    fn packed_names(root: &Path, compression: Compression) -> Vec<String> {
        let (bytes, _) = compress_student_files(
            &EverythingIsStudentFilePolicy::new(root).unwrap(),
            root,
            compression,
            true,
            false,
            10,
        )
        .unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect()
    }

    #[test]
    fn packing_skips_conflict_copies() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("ex");
        for name in [
            "src/Main.java",
            "src/Main-DESKTOP-AB12CD3-1.java",
            "src/Main-OLD.java",
            "src/~$Main.java",
            "src/my-file.py",
            "src/Util.sync-conflict-20240131-101010-ABC.java",
        ] {
            file_util::write_to_file(b"x", root.join(name)).unwrap();
        }
        let names = packed_names(&root, Compression::Zip);
        assert!(names.contains(&"ex/src/Main.java".to_string()));
        assert!(names.contains(&"ex/src/my-file.py".to_string()));
        assert!(names.contains(&"ex/src/Main-OLD.java".to_string()));
        assert_eq!(
            names.iter().filter(|n| !n.ends_with('/')).count(),
            3,
            "{names:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn packing_does_not_follow_symlinks() {
        let temp = tempdir().unwrap();
        let outside = temp.path().join("secret.txt");
        file_util::write_to_file(b"secret", &outside).unwrap();
        let root = temp.path().join("ex");
        file_util::write_to_file(b"x", root.join("src/Main.java")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("src/leak.txt")).unwrap();
        std::os::unix::fs::symlink(temp.path(), root.join("src/dirlink")).unwrap();
        for compression in [Compression::Zip, Compression::Tar, Compression::TarZstd] {
            let (bytes, _) = compress_student_files(
                &EverythingIsStudentFilePolicy::new(&root).unwrap(),
                &root,
                compression,
                true,
                false,
                10,
            )
            .unwrap();
            let unpacked = tempdir().unwrap();
            tmc_langs_framework::Archive::new(Cursor::new(bytes), compression)
                .unwrap()
                .extract(unpacked.path())
                .unwrap();
            assert!(unpacked.path().join("ex/src/Main.java").is_file());
            assert!(
                !unpacked.path().join("ex/src/leak.txt").exists(),
                "{compression}"
            );
            assert!(
                !unpacked.path().join("ex/src/dirlink").exists(),
                "{compression}"
            );
        }
    }
}
