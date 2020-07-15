//! Contains a function for creating a tarball from a project.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;
use tmc_langs_framework::{Error, Result};
use walkdir::WalkDir;

/// Creates a tarball from the project dir, also adding in tmc_langs and tmcrun.
pub fn create_tar_from_project(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<()> {
    log::debug!(
        "creating tar from {} to {} with tmc-langs at {} and tmcrun at {}",
        project_dir.display(),
        target_location.display(),
        tmc_langs.display(),
        tmcrun.display()
    );
    let file = File::create(target_location)
        .map_err(|e| Error::CreateFile(target_location.to_path_buf(), e))?;
    let mut tar = Builder::new(file);

    let project_name = Path::new(
        project_dir
            .file_name()
            .ok_or(Error::NoFileName(project_dir.to_path_buf()))?,
    );
    let root = project_dir.parent().unwrap_or_else(|| Path::new(""));
    add_dir_to_project(&mut tar, &project_dir, project_dir, &project_name)?;
    add_dir_to_project(&mut tar, &tmc_langs, root, &project_name)?;
    add_dir_to_project(&mut tar, &tmcrun, root, &project_name)?;
    tar.finish().map_err(|e| Error::TarFinish(e))?;
    Ok(())
}

fn add_dir_to_project<W: Write>(
    tar: &mut Builder<W>,
    source: &Path,
    root: &Path,
    project_name: &Path,
) -> Result<()> {
    // silently skips over errors
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            let path_in_project = entry.path().strip_prefix(root).unwrap();
            let path_in_tar: PathBuf = project_name.join(path_in_project);
            log::trace!("appending {:?} as {:?}", entry.path(), path_in_tar);
            tar.append_path_with_name(entry.path(), path_in_tar)
                .map_err(|e| Error::TarAppend(e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashSet;
    use tar::Archive;
    use tempfile::tempdir;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn creates_tar_from_project() {
        init();

        let temp = tempdir().unwrap();
        let tar_path = temp.path().join("tar.tar");
        create_tar_from_project(
            Path::new("tests/data/project"),
            Path::new("tests/data/tmc-langs"),
            Path::new("tests/data/tmcrun"),
            &tar_path,
        )
        .unwrap();

        let tar = File::open(tar_path).unwrap();
        let mut archive = Archive::new(tar);
        let mut paths = HashSet::new();
        for file in archive.entries().unwrap() {
            paths.insert(file.unwrap().header().path().unwrap().into_owned());
        }
        log::debug!("{:?}", paths);
        assert!(paths.contains(Path::new("project/projectfile")));
        assert!(paths.contains(Path::new("project/tmc-langs/langsfile")));
        assert!(paths.contains(Path::new("project/tmcrun/runfile")));
    }
}
