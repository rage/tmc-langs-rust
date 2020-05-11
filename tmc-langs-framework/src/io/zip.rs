use super::super::Result;
use super::StudentFilePolicy;
use log::debug;
use std::fs::{self, File};
use std::io::{BufWriter, Bytes, Read, Seek, Write};
use std::path::Path;
use walkdir::WalkDir;
pub use zip::result::ZipError;
use zip::{ZipArchive, ZipWriter};

pub struct UnzipResult {}

pub fn student_file_aware_unzip(
    policy: Box<dyn StudentFilePolicy>,
    zip: &Path,
    target: &Path,
) -> Result<UnzipResult> {
    let file = File::open(zip)?;
    let mut zip_archive = ZipArchive::new(file)?;
    // find project dir
    let project_dir = find_project_dir(&mut zip_archive)?;
    let project_path = Path::new(&project_dir);

    for i in 0..zip_archive.len() {
        let file = zip_archive.by_index(i)?;
        let file_path = file.sanitized_name();
        if !file_path.starts_with(project_path) {
            debug!("skip {}, not in project dir", file.name());
            continue;
        }
        let path_in_target = target.join(&file_path);
        debug!("processing {:?} -> {:?}", file_path, path_in_target);

        if file.is_dir() {
            if policy.is_student_file(&path_in_target, &target)? {
                debug!("creating {:?}", path_in_target);
                fs::create_dir_all(&path_in_target)?;
            }
        } else {
            let mut write = true;
            let file_contents = file.bytes().collect::<std::result::Result<Vec<_>, _>>()?;
            if path_in_target.exists() {
                let target_file = File::open(&path_in_target)?;
                let target_file_contents = target_file
                    .bytes()
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if file_contents == target_file_contents {
                    write = false;
                } else {
                    // check "allowed to unzip"? = !is student file || is updating forced ?
                    if policy.is_student_file(&path_in_target, &target)?
                        && !policy.is_updating_forced(&path_in_target)?
                    {
                        // student file and not a forced update
                        write = false;
                    }
                }
            }
            if write {
                let mut overwrite_target = File::create(path_in_target)?;
                overwrite_target.write_all(&file_contents)?;
            }
        }
    }

    // overwrite .tmcprojectyml
    let yml_path_in_zip = project_path.join(".tmcproject.yml");
    let yml_path_in_target = target.join(&yml_path_in_zip);
    let yml_zipped = zip_archive.by_name(yml_path_in_zip.to_str().expect("non-UTF-8 name"))?;
    let yml_file = File::create(yml_path_in_target)?;
    let mut yml_writer = BufWriter::new(yml_file);
    for byte in yml_zipped.bytes() {
        let byte = byte?;
        yml_writer.write_all(&[byte])?;
    }

    Ok(UnzipResult {})
}

fn find_project_dir<R: Read + Seek>(zip_archive: &mut ZipArchive<R>) -> Result<String> {
    for i in 0..zip_archive.len() {
        let file = zip_archive.by_index(i)?;
        let file_path = file.sanitized_name();
        let file_name = file_path
            .file_name()
            .and_then(|o| o.to_str())
            .unwrap_or_default();
        if file.is_dir() && (file_name == "nbproject" || file_name == "src" || file_name == "test")
            || file.is_file()
                && (file_name == "pom.xml" || file_name == ".idea" || file_name == "Makefile")
        {
            debug!("found project dir {}", file.name());
            return Ok(file.name().to_string());
        }
    }
    todo!("no project dir found in zip")
}

pub fn student_file_aware_zip(
    policy: Box<dyn StudentFilePolicy>,
    root_directory: &Path,
) -> Vec<u8> {
    for entry in WalkDir::new(root_directory)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().is_dir() {}
    }
    todo!()
}

#[cfg(test)]
mod test {
    use super::super::EverythingIsStudentFilePolicy;
    use super::*;
    use std::collections::HashSet;
    use tempdir::TempDir;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test() {
        init();

        let temp = TempDir::new("test").unwrap();
        let zip_path = Path::new("testdata/zip.zip");
        student_file_aware_unzip(
            Box::new(EverythingIsStudentFilePolicy {}),
            zip_path,
            temp.path(),
        )
        .unwrap();

        let mut paths = HashSet::new();
        for entry in walkdir::WalkDir::new(temp.path()) {
            let entry = entry.unwrap();
            paths.insert(entry.path().to_owned());
        }
        assert!(paths.contains(&temp.path().join("outer/src/file.py")));
        assert!(paths.contains(&temp.path().join("outer/src/.tmcproject.yml")));
        assert!(!paths.contains(&temp.path().join("other/some file")));
    }
}
