//! Course refresher.

use crate::{error::LangsError, progress_reporter};
use md5::Context;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use zip::write::SimpleFileOptions;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{TmcCommand, TmcProjectYml};
use tmc_langs_util::{deserialize, file_util};
use walkdir::WalkDir;

#[cfg(unix)]
pub type ModeBits = nix::sys::stat::mode_t;

/// Data from a finished course refresh.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct RefreshData {
    pub new_cache_path: PathBuf,
    #[cfg_attr(feature = "ts-rs", ts(type = "object"))]
    pub course_options: Mapping,
    pub exercises: Vec<RefreshExercise>,
}

/// An exercise from a finished course refresh.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct RefreshExercise {
    name: String,
    checksum: String,
    points: Vec<String>,
    #[serde(skip)]
    path: PathBuf,
    sandbox_image: String,
    #[cfg_attr(feature = "ts-rs", ts(type = "TmcProjectYml | null"))]
    tmcproject_yml: Option<TmcProjectYml>,
}

/// Used by tmc-server. Refreshes the course.
pub fn refresh_course(
    course_name: String,
    course_cache_path: PathBuf,
    source_url: String,
    git_branch: String,
    cache_root: PathBuf,
) -> Result<RefreshData, LangsError> {
    log::info!("refreshing course {}", course_name);
    start_stage(10, "Refreshing course");

    // create new cache path
    let old_version = course_cache_path
        .to_str()
        .and_then(|s| s.split('-').last())
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| LangsError::InvalidCachePath(course_cache_path.clone()))?;
    let new_cache_path = cache_root.join(format!("{}-{}", course_name, old_version + 1));
    log::info!("next cache path: {}", new_cache_path.display());

    if new_cache_path.exists() {
        log::info!("clearing new cache path at {}", new_cache_path.display());
        file_util::remove_dir_all(&new_cache_path)?;
    }
    file_util::create_dir_all(&new_cache_path)?;
    progress_stage("Created new cache dir");

    // initialize new clone path and verify directory names
    let new_clone_path = new_cache_path.join("clone");
    let old_clone_path = course_cache_path.join("clone");
    initialize_new_cache_clone(
        &new_cache_path,
        &new_clone_path,
        &old_clone_path,
        &source_url,
        &git_branch,
    )?;
    check_directory_names(&new_clone_path)?;
    progress_stage("Updated repository");

    let course_options = get_course_options(&new_clone_path, &course_name)?;
    progress_stage("Fetched course options");

    let new_solution_path = new_cache_path.join("solution");
    let new_stub_path = new_cache_path.join("stub");

    let exercise_dirs = super::find_exercise_directories(&new_clone_path)?
        .into_iter()
        .map(|ed| {
            ed.strip_prefix(&new_clone_path)
                .expect("exercise directories are inside new_clone_path")
                .to_path_buf()
        })
        .collect::<Vec<_>>();

    // collect .tmcproject.ymls and merge the root config with each exercise's, if any
    let root_tmcproject_yml = TmcProjectYml::load(&new_clone_path)?;
    let exercise_dirs_and_tmcprojects =
        get_and_merge_tmcproject_configs(root_tmcproject_yml, &new_clone_path, exercise_dirs)?;
    progress_stage("Merged .tmcproject.yml files in exercise directories to the root file, if any");

    // make_solutions
    log::info!("preparing solutions to {}", new_solution_path.display());
    for (exercise, merged_tmcproject) in &exercise_dirs_and_tmcprojects {
        // save merged config to solution
        let dest_root = new_solution_path.join(exercise);
        super::prepare_solution(&new_clone_path.join(exercise), &dest_root)?;
        if let Some(merged_tmcproject) = merged_tmcproject {
            merged_tmcproject.save_to_dir(&dest_root)?;
        }
    }
    progress_stage("Prepared solutions");

    // make_stubs
    log::info!("preparing stubs to {}", new_stub_path.display());
    for (exercise, merged_tmcproject) in &exercise_dirs_and_tmcprojects {
        // save merged config to stub
        let dest_root = new_stub_path.join(exercise);
        super::prepare_stub(&new_clone_path.join(exercise), &dest_root)?;
        if let Some(merged_tmcproject) = merged_tmcproject {
            merged_tmcproject.save_to_dir(&dest_root)?;
        }
    }
    progress_stage("Prepared stubs");

    let exercises = get_exercises(
        exercise_dirs_and_tmcprojects,
        &new_clone_path,
        &new_stub_path,
    )?;
    progress_stage("Located exercises");

    // make_zips_of_solutions
    let new_solution_zip_path = new_cache_path.join("solution_zip");
    execute_zip(&exercises, &new_solution_path, &new_solution_zip_path)?;
    progress_stage("Compressed solutions");

    // make_zips_of_stubs
    let new_stub_zip_path = new_cache_path.join("stub_zip");
    log::info!(
        "compressing stubs from {} to {}",
        new_stub_path.display(),
        new_stub_zip_path.display()
    );
    execute_zip(&exercises, &new_stub_path, &new_stub_zip_path)?;
    progress_stage("Compressed stubs");

    // make sure the new cache path is readable by anyone
    set_permissions(&new_cache_path)?;

    finish_stage("Refreshed course");
    Ok(RefreshData {
        new_cache_path,
        course_options,
        exercises,
    })
}

/// Checks old_cache_path/clone for a git repo.
/// If found, copies it to course_clone_path fetches origin from course_source_url, checks out origin/course_git_branch, cleans and checks out the repo.
/// If not found or found but one of the git commands causes an error, deletes course_clone_path and clones course_git_branch from course_source_url there.
/// NOP during testing.
fn initialize_new_cache_clone(
    new_course_root: &Path,
    new_clone_path: &Path,
    old_clone_path: &Path,
    course_source_url: &str,
    course_git_branch: &str,
) -> Result<(), LangsError> {
    log::info!("initializing repository at {}", new_clone_path.display());

    if old_clone_path.join(".git").exists() {
        log::info!(
            "trying to copy clone from previous cache at {}",
            old_clone_path.display()
        );

        // closure to collect any error that occurs during the process
        let copy_and_update_repository = || -> Result<(), LangsError> {
            file_util::copy(old_clone_path, new_course_root)?;

            let run_git = |args: &[&str]| {
                TmcCommand::piped("git")
                    .with(|e| e.cwd(new_clone_path).args(args))
                    .output_with_timeout_checked(Duration::from_secs(60 * 2))
            };

            run_git(&["remote", "set-url", "origin", course_source_url])?;
            run_git(&["fetch", "origin"])?;
            run_git(&["checkout", &format!("origin/{course_git_branch}")])?;
            run_git(&["clean", "-df"])?;
            run_git(&["checkout", "."])?;
            Ok(())
        };
        match copy_and_update_repository() {
            Ok(_) => {
                log::info!("updated repository");
                return Ok(());
            }
            Err(error) => {
                log::warn!("failed to update repository: {}", error);

                file_util::remove_dir_all(new_clone_path)?;
            }
        }
    };

    log::info!("could not copy from previous cache, cloning");

    // clone_repository
    TmcCommand::piped("git")
        .with(|e| {
            e.args(&["clone", "-q", "-b"])
                .arg(course_git_branch)
                .arg(course_source_url)
                .arg(new_clone_path)
        })
        .output_with_timeout_checked(Duration::from_secs(60 * 2))?;
    Ok(())
}

/// Makes sure no directory directly under path is an exercise directory containing a dash in the relative path from path to the dir.
/// A dash is used as a special delimiter.
fn check_directory_names(path: &Path) -> Result<(), LangsError> {
    log::info!("checking directory names for dashes");

    // exercise directories in canonicalized form
    for exercise_dir in super::find_exercise_directories(path)? {
        let relative = exercise_dir
            .strip_prefix(path)
            .expect("the exercise dirs are all inside the path");
        if relative.to_string_lossy().contains('-') {
            return Err(LangsError::InvalidDirectory(exercise_dir));
        }
    }
    Ok(())
}

fn get_and_merge_tmcproject_configs(
    root_tmcproject: Option<TmcProjectYml>,
    clone_path: &Path,
    exercise_dirs: Vec<PathBuf>,
) -> Result<Vec<(PathBuf, Option<TmcProjectYml>)>, LangsError> {
    let mut res = vec![];
    for exercise_dir in exercise_dirs {
        let target_dir = clone_path.join(&exercise_dir);
        let exercise_tmcproject = TmcProjectYml::load(&target_dir)?;
        match (&root_tmcproject, exercise_tmcproject) {
            (Some(root), Some(mut exercise)) => {
                exercise.merge(root.clone());
                res.push((exercise_dir, Some(exercise)));
            }
            (Some(root), None) => {
                res.push((exercise_dir, Some(root.clone())));
            }
            (None, Some(exercise)) => res.push((exercise_dir, Some(exercise))),
            (None, None) => res.push((exercise_dir, None)),
        }
    }
    Ok(res)
}

/// Checks for a course_clone_path/course_options.yml
/// If found, course-specific options are merged into it and it is returned.
/// Else, an empty mapping is returned.
fn get_course_options(course_clone_path: &Path, course_name: &str) -> Result<Mapping, LangsError> {
    log::info!(
        "collecting course options for {} in {}",
        course_name,
        course_clone_path.display()
    );

    let options_file = course_clone_path.join("course_options.yml");
    if options_file.exists() {
        let file = file_util::open_file(&options_file)?;
        let course_options: Mapping = deserialize::yaml_from_reader(file)
            .map_err(|e| LangsError::DeserializeYaml(options_file, e))?;
        Ok(course_options)
    } else {
        Ok(Mapping::new())
    }
}

/// Finds exercise directories, and converts the directories to "exercise names" by swapping the separators for dashes.
/// Also calculates checksums and fetches points for all
fn get_exercises(
    exercise_dirs_and_tmcprojects: Vec<(PathBuf, Option<TmcProjectYml>)>,
    course_clone_path: &Path,
    course_stub_path: &Path,
) -> Result<Vec<RefreshExercise>, LangsError> {
    log::info!("finding exercise checksums and points");

    let exercises = exercise_dirs_and_tmcprojects
        .into_iter()
        .map(|(exercise_dir, tmcproject_yml)| {
            log::debug!(
                "processing points and checksum for {}",
                exercise_dir.display()
            );
            let name = exercise_dir.to_string_lossy().replace('/', "-");
            let checksum = calculate_checksum(&course_stub_path.join(&exercise_dir))?;
            let exercise_path = course_clone_path.join(&exercise_dir);
            let points = super::get_available_points(&exercise_path)?;

            let sandbox_image = if let Some(image_override) = tmcproject_yml
                .as_ref()
                .and_then(|y| y.sandbox_image.as_ref())
            {
                image_override.clone()
            } else {
                crate::get_default_sandbox_image(&exercise_path)?.to_string()
            };

            Ok(RefreshExercise {
                name,
                points,
                checksum,
                path: exercise_dir,
                sandbox_image,
                tmcproject_yml,
            })
        })
        .collect::<Result<_, LangsError>>()?;
    Ok(exercises)
}

fn calculate_checksum(exercise_dir: &Path) -> Result<String, LangsError> {
    let mut digest = Context::new();

    // order filenames for a consistent hash
    for entry in WalkDir::new(exercise_dir)
        .min_depth(1) // do not hash the directory itself ('.')
        .sort_by(|a, b| a.file_name().cmp(b.file_name()))
    {
        let entry = entry?;
        let relative = entry
            .path()
            .strip_prefix(exercise_dir)
            .expect("the entry is inside the exercise dir");
        let string = relative.as_os_str().to_string_lossy();
        digest.consume(string.as_ref());
        if entry.path().is_file() {
            let file = file_util::read_file(entry.path())?;
            digest.consume(file);
        }
    }

    // convert the digest into a hex string
    let digest = digest.compute();
    Ok(format!("{digest:x}"))
}

fn execute_zip(
    course_exercises: &[RefreshExercise],
    root_path: &Path,
    zip_dir: &Path,
) -> Result<(), LangsError> {
    log::info!(
        "compressing exercises from from {} to {}",
        root_path.display(),
        zip_dir.display()
    );

    file_util::create_dir_all(zip_dir)?;
    for exercise in course_exercises {
        let exercise_root = root_path.join(&exercise.path);
        let zip_file_path = zip_dir.join(format!("{}.zip", exercise.name));

        let mut writer = zip::ZipWriter::new(file_util::create_file(zip_file_path)?);
        for entry in WalkDir::new(exercise_root) {
            let entry = entry?;
            let relative_path = entry
                .path()
                .strip_prefix(root_path)
                .expect("entries are inside root_path");

            if entry.path().is_file() {
                writer.start_file(
                    relative_path.to_string_lossy(),
                    SimpleFileOptions::default().unix_permissions(0o755),
                )?;
                let bytes = file_util::read_file(entry.path())?;
                writer.write_all(&bytes).map_err(LangsError::ZipWrite)?;
            } else {
                // java-langs expects directories to have their own entries
                writer.start_file(
                    relative_path.join("").to_string_lossy(), // java-langs expects directory entries to have a trailing slash
                    SimpleFileOptions::default().unix_permissions(0o755),
                )?;
            }
        }
        writer.finish()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path) -> Result<(), LangsError> {
    // NOP on non-Unix platforms
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path) -> Result<(), LangsError> {
    use nix::sys::stat;
    use std::os::unix::io::AsRawFd;

    log::info!("setting permissions in {}", path.display());

    let chmod: ModeBits = 0o775; // octal, read and execute permissions for all users
    for entry in WalkDir::new(path) {
        let entry = entry?;
        let file = file_util::open_file(entry.path())?;
        stat::fchmod(
            file.as_raw_fd(),
            stat::Mode::from_bits(chmod).ok_or(LangsError::NixFlag(chmod))?,
        )
        .map_err(|e| LangsError::NixPermissionChange(path.to_path_buf(), e))?;
    }

    Ok(())
}

fn start_stage(steps: u32, message: impl Into<String>) {
    progress_reporter::start_stage::<()>(steps, message.into(), None)
}

fn progress_stage(message: impl Into<String>) {
    progress_reporter::progress_stage::<()>(message.into(), None)
}

fn finish_stage(message: impl Into<String>) {
    progress_reporter::finish_stage::<()>(message.into(), None)
}

#[cfg(test)]
#[cfg(unix)] // not used on windows
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use crate::find_exercise_directories;
    use serde_yaml::Value;
    use std::io::Read;
    use tempfile::tempdir;

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

    #[test]
    fn checks_directory_names() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "course/valid_part/valid_ex/setup.py", "");
        assert!(check_directory_names(&temp.path().join("course")).is_ok());

        let course = tempfile::tempdir().unwrap();
        file_to(course.path(), "course/part1/invalid-ex1/setup.py", "");
        assert!(check_directory_names(&course.path().join("course")).is_err());

        let course = tempfile::tempdir().unwrap();
        file_to(course.path(), "course/invalid-part/valid_ex/setup.py", "");
        assert!(check_directory_names(&course.path().join("course")).is_err());
    }

    #[test]
    fn gets_course_options() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "course_options.yml", "option: true");
        let options = get_course_options(temp.path(), "some course").unwrap();
        assert_eq!(options.len(), 1);
        assert!(options
            .get(&Value::String("option".to_string()))
            .unwrap()
            .as_bool()
            .unwrap())
    }

    #[test]
    fn gets_exercises() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "course/part1/ex1/setup.py", "");
        file_to(
            &temp,
            "course/part1/ex1/test/test.py",
            "@points('1') @points('2')",
        );
        let exercise_dirs = find_exercise_directories(&temp.path().join("course"))
            .unwrap()
            .into_iter()
            .map(|ed| {
                (
                    ed.strip_prefix(&temp.path().join("course"))
                        .unwrap()
                        .to_path_buf(),
                    None,
                )
            })
            .collect();
        let exercises = get_exercises(
            exercise_dirs,
            &temp.path().join("course"),
            &temp.path().join("course"),
        )
        .unwrap();
        assert_eq!(exercises.len(), 1);
        assert_eq!(exercises[0].path, Path::new("part1/ex1"));
        assert_eq!(exercises[0].points.len(), 2);
        assert_eq!(exercises[0].points[0], "1");
        assert_eq!(exercises[0].points[1], "2");
        assert_eq!(exercises[0].checksum, "129e7e898698465c4f24494219f06df9");
    }

    #[test]
    fn executes_zip() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "clone/part1/ex1/setup.py", "");
        file_to(&temp, "clone/part1/ex2/setup.py", "");
        file_to(&temp, "clone/part2/ex1/setup.py", "");
        file_to(&temp, "clone/part2/ex2/setup.py", "");
        file_to(&temp, "clone/part2/ex2/dir/subdir/file", "");
        file_to(&temp, "clone/part2/ex2/.tmcproject.yml", "some: 'yaml'");
        file_to(&temp, "stub/part1/ex1/setup.py", "");
        file_to(&temp, "stub/part1/ex2/setup.py", "");
        file_to(&temp, "stub/part2/ex1/setup.py", "");
        file_to(&temp, "stub/part2/ex2/setup.py", "");
        file_to(&temp, "stub/part2/ex2/dir/subdir/file", "some file");
        file_to(&temp, "stub/part2/ex2/.tmcproject.yml", "some: 'yaml'");

        let exercise_dirs = find_exercise_directories(&temp.path().join("clone"))
            .unwrap()
            .into_iter()
            .map(|ed| {
                (
                    ed.strip_prefix(&temp.path().join("clone"))
                        .unwrap()
                        .to_path_buf(),
                    None,
                )
            })
            .collect();
        let exercises = get_exercises(
            exercise_dirs,
            &temp.path().join("clone"),
            &temp.path().join("stub"),
        )
        .unwrap();

        execute_zip(&exercises, &temp.path().join("stub"), temp.path()).unwrap();

        let zip = temp.path().join("part1-ex1.zip");
        assert!(zip.exists());
        let zip = temp.path().join("part1-ex2.zip");
        assert!(zip.exists());
        let zip = temp.path().join("part2-ex1.zip");
        assert!(zip.exists());
        let zip = temp.path().join("part2-ex2.zip");
        assert!(zip.exists());

        let mut fz = zip::ZipArchive::new(file_util::open_file(&zip).unwrap()).unwrap();
        for i in fz.file_names() {
            log::debug!("{}", i);
        }
        assert!(fz
            .by_name(
                &Path::new("part2")
                    .join("ex2")
                    .join("dir")
                    .join("subdir")
                    .join("")
                    .to_string_lossy(),
            )
            .is_ok()); // directories have their own entries with trailing slashes
        let mut file = fz
            .by_name(
                &Path::new("part2")
                    .join("ex2")
                    .join("dir")
                    .join("subdir")
                    .join("file")
                    .to_string_lossy(),
            )
            .unwrap(); // other files have their stub contents
        let mut buf = String::new();
        file.read_to_string(&mut buf).unwrap();
        drop(file);

        assert_eq!(buf, "some file");
        let mut file = fz
            .by_name(
                &Path::new("part2")
                    .join("ex2")
                    .join(".tmcproject.yml")
                    .to_string_lossy(),
            )
            .unwrap();
        let mut buf = String::new();
        file.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "some: 'yaml'");
    }

    #[test]
    #[ignore = "issues in CI, maybe due to the user ID"]
    fn sets_permissions() {
        init();

        let temp = tempdir().unwrap();
        file_to(&temp, "file", "contents");

        set_permissions(temp.path()).unwrap();
    }

    #[test]
    fn checksum_matches_old_implementation() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/test.py",
            r#"@points("test_point")
@points("ex_and_test_point")
"#,
        );
        file_to(
            &temp,
            ".hidden file that should be included in the hash",
            "",
        );
        file_to(&temp, "invalid-but-not-dir", "");
        file_to(&temp, "setup.py", "");

        let checksum = calculate_checksum(temp.path()).unwrap();
        assert_eq!(checksum, "6cacf02f21f9242674a876954132fb11");
    }

    #[test]
    fn merges_tmcproject_configs() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let exap = PathBuf::from("exa");
        let exap_path = temp.path().join(&exap);
        file_util::create_dir(&exap_path).unwrap();
        let exbp = PathBuf::from("exb");
        let exbp_path = temp.path().join(&exbp);
        file_util::create_dir(&exbp_path).unwrap();

        let root = TmcProjectYml {
            tests_timeout_ms: Some(1234),
            fail_on_valgrind_error: Some(true),
            ..Default::default()
        };
        let tpya = TmcProjectYml {
            tests_timeout_ms: Some(2345),
            ..Default::default()
        };
        tpya.save_to_dir(&exap_path).unwrap();
        let tpyb = TmcProjectYml {
            fail_on_valgrind_error: Some(false),
            ..Default::default()
        };
        tpyb.save_to_dir(&exbp_path).unwrap();
        let exercise_dirs = vec![exap, exbp];

        let dirs_configs =
            get_and_merge_tmcproject_configs(Some(root), temp.path(), exercise_dirs).unwrap();

        let (_, tpya) = &dirs_configs
            .iter()
            .find(|(p, _)| p.ends_with("exa"))
            .unwrap();
        let tpya = tpya.as_ref().unwrap();
        assert_eq!(tpya.tests_timeout_ms, Some(2345));
        assert_eq!(tpya.fail_on_valgrind_error, Some(true));

        let (_, tpyb) = &dirs_configs
            .iter()
            .find(|(p, _)| p.ends_with("exb"))
            .unwrap();
        let tpyb = tpyb.as_ref().unwrap();
        assert_eq!(tpyb.tests_timeout_ms, Some(1234));
        assert_eq!(tpyb.fail_on_valgrind_error, Some(false));
    }
}
