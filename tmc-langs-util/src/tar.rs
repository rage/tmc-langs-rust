use log::debug;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;
use tmc_langs_framework::Result;
use walkdir::WalkDir;

pub fn create_tar_from_project(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<()> {
    let file = File::create(target_location)?;
    let mut t = Builder::new(file);

    let project_skips = project_dir.components().count() - 1;
    let project_name: PathBuf = project_dir.components().skip(project_skips).collect();
    add_dir_to_project(&mut t, &project_dir, project_skips, &Path::new(""))?;
    let langs_skips = tmc_langs.components().count() - 1;
    add_dir_to_project(&mut t, &tmc_langs, langs_skips, &project_name)?;
    let run_skips = tmcrun.components().count() - 1;
    add_dir_to_project(&mut t, &tmcrun, run_skips, &project_name)?;
    t.finish()?;
    Ok(())
}

fn add_dir_to_project<W: Write>(
    tar: &mut Builder<W>,
    source: &Path,
    skips: usize,
    project_name: &Path,
) -> Result<()> {
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            let path_in_project: PathBuf = entry.path().iter().skip(skips).collect();
            let path_in_tar: PathBuf = project_name.join(path_in_project);
            debug!("appending {:?} as {:?}", entry.path(), path_in_tar);
            tar.append_path_with_name(entry.path(), path_in_tar)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashSet;
    use tar::Archive;
    use tempdir::TempDir;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn creates_tar_from_project() {
        init();

        let temp = TempDir::new("creates_tar_from_project").unwrap();
        let tar_path = temp.path().join("tar.tar");
        create_tar_from_project(
            Path::new("testdata/project"),
            Path::new("testdata/tmc-langs"),
            Path::new("testdata/tmcrun"),
            &tar_path,
        )
        .unwrap();

        let tar = File::open(tar_path).unwrap();
        let mut archive = Archive::new(tar);
        let mut paths = HashSet::new();
        for file in archive.entries().unwrap() {
            paths.insert(file.unwrap().header().path().unwrap().into_owned());
        }
        debug!("{:?}", paths);
        assert!(paths.contains(Path::new("project/projectfile")));
        assert!(paths.contains(Path::new("project/tmc-langs/langsfile")));
        assert!(paths.contains(Path::new("project/tmcrun/runfile")));
    }
}
