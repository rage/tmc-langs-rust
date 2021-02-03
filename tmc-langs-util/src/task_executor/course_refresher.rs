//! Course refresher.

use crate::{
    error::UtilError,
    progress_reporter::{ProgressReporter, StatusUpdate},
    task_executor,
};
use md5::Context;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::io::Write;
use std::path::{Path, PathBuf};
use tmc_langs_framework::{command::TmcCommand, file_util, subprocess::Redirection};
use walkdir::WalkDir;

#[cfg(unix)]
pub type ModeBits = nix::sys::stat::mode_t;
#[cfg(not(unix))]
pub type ModeBits = u32;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RefreshData {
    pub new_cache_path: PathBuf,
    pub course_options: Mapping,
    pub exercises: Vec<RefreshExercise>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshExercise {
    name: String,
    checksum: String,
    points: Vec<String>,
    #[serde(skip)]
    path: PathBuf,
}

struct CourseRefresher {
    progress_reporter: ProgressReporter<'static, ()>,
}

impl CourseRefresher {
    pub fn new(
        progress_report: impl 'static
            + Sync
            + Send
            + Fn(StatusUpdate<()>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            progress_reporter: ProgressReporter::new(progress_report),
        }
    }

    pub fn refresh_course(
        self,
        course_name: String,
        course_cache_path: PathBuf,
        source_url: String,
        git_branch: String,
        cache_root: PathBuf,
    ) -> Result<RefreshData, UtilError> {
        log::info!("refreshing course {}", course_name);
        self.progress_reporter.start_timer();

        // sets the total amount of progress steps properly
        self.progress_reporter.increment_progress_steps(13);

        // create new cache path
        let old_version = course_cache_path
            .to_str()
            .unwrap()
            .split('-')
            .last()
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let new_cache_path = cache_root.join(format!("{}-{}", course_name, old_version + 1));
        if new_cache_path.exists() {
            log::info!("clearing new cache path at {}", new_cache_path.display());
            file_util::remove_dir_all(&new_cache_path)?;
        }
        file_util::create_dir_all(&new_cache_path)?;
        self.progress_reporter
            .finish_step("Created new cache dir".to_string(), None)?;

        // initialize new clone path and verify directory names
        let new_clone_path = new_cache_path.join("clone");
        log::info!("updating repository to {}", new_clone_path.display());
        let old_clone_path = course_cache_path.join("clone");
        update_or_clone_repository(&new_clone_path, &old_clone_path, &source_url, &git_branch)?;
        check_directory_names(&new_clone_path)?;
        self.progress_reporter
            .finish_step("Updated repository".to_string(), None)?;

        log::info!("updating course options");
        let course_options = get_course_options(&new_clone_path, &course_name)?;
        self.progress_reporter
            .finish_step("Updated course options".to_string(), None)?;

        let new_solution_path = new_cache_path.join("solution");
        let new_stub_path = new_cache_path.join("stub");

        let exercise_dirs = task_executor::find_exercise_directories(&new_clone_path)?
            .into_iter()
            .map(|ed| ed.strip_prefix(&new_clone_path).unwrap().to_path_buf())
            .collect();

        // make_solutions
        log::info!("preparing solutions to {}", new_solution_path.display());
        for exercise in &exercise_dirs {
            task_executor::prepare_solution(
                &new_clone_path.join(&exercise),
                &new_solution_path.join(&exercise),
            )?;
        }
        self.progress_reporter
            .finish_step("Prepared solutions".to_string(), None)?;

        // make_stubs
        log::info!("preparing stubs to {}", new_stub_path.display());
        for exercise in &exercise_dirs {
            task_executor::prepare_stub(
                &new_clone_path.join(&exercise),
                &new_stub_path.join(&exercise),
            )?;
        }
        self.progress_reporter
            .finish_step("Prepared stubs".to_string(), None)?;

        // find exercises in new clone path
        log::info!("finding exercises");
        // (exercise name, exercise path)
        let exercises = get_exercises(exercise_dirs, &new_clone_path, &new_stub_path)?;
        self.progress_reporter
            .finish_step("Located exercises".to_string(), None)?;

        // make_zips_of_solutions
        let new_solution_zip_path = new_cache_path.join("solution_zip");
        log::info!("compressing solutions");
        execute_zip(&exercises, &new_solution_path, &new_solution_zip_path)?;
        self.progress_reporter
            .finish_step("Compressed solutions".to_string(), None)?;

        // make_zips_of_stubs
        let new_stub_zip_path = new_cache_path.join("stub_zip");
        log::info!("compressing stubs");
        execute_zip(&exercises, &new_stub_path, &new_stub_zip_path)?;
        self.progress_reporter
            .finish_step("Compressed stubs".to_string(), None)?;

        // make sure the new cache path is readable by anyone
        set_permissions(&new_cache_path)?;

        self.progress_reporter
            .finish_step("Refreshed course".to_string(), None)?;
        Ok(RefreshData {
            new_cache_path,
            course_options,
            exercises,
        })
    }
}

/// Refreshes a course...
pub fn refresh_course(
    course_name: String,
    course_cache_path: PathBuf,
    source_url: String,
    git_branch: String,
    cache_root: PathBuf,
    progress_reporter: impl 'static
        + Sync
        + Send
        + Fn(StatusUpdate<()>) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
) -> Result<RefreshData, UtilError> {
    let course_refresher = CourseRefresher::new(progress_reporter);
    course_refresher.refresh_course(
        course_name,
        course_cache_path,
        source_url,
        git_branch,
        cache_root,
    )
}

/// Checks old_cache_path/clone for a git repo.
/// If found, copies it to course_clone_path fetches origin from course_source_url, checks out origin/course_git_branch, cleans and checks out the repo.
/// If not found or found but one of the git commands causes an error, deletes course_clone_path and clones course_git_branch from course_source_url there.
/// NOP during testing.
fn update_or_clone_repository(
    new_clone_path: &Path,
    old_clone_path: &Path,
    course_source_url: &str,
    course_git_branch: &str,
) -> Result<(), UtilError> {
    if old_clone_path.join(".git").exists() {
        // Try a fast path: copy old clone and git fetch new stuff

        // closure to collect any error that occurs during the process
        let copy_and_update_repository = || -> Result<(), UtilError> {
            file_util::copy(old_clone_path, new_clone_path)?;

            let run_git = |args: &[&str]| {
                TmcCommand::new("git".to_string())
                    .with(|e| {
                        e.cwd(new_clone_path)
                            .args(args)
                            .stdout(Redirection::Pipe)
                            .stderr(Redirection::Pipe)
                    })
                    .output_checked()
            };

            let clone_path_str = new_clone_path.to_str().unwrap();
            run_git(&[
                "-C",
                clone_path_str,
                "remote",
                "set-url",
                "origin",
                course_source_url,
            ])?;
            run_git(&["-C", clone_path_str, "fetch", "origin"])?;
            run_git(&[
                "-C",
                clone_path_str,
                "checkout",
                &format!("origin/{}", course_git_branch),
            ])?;
            run_git(&["-C", clone_path_str, "clean", "-df"])?;
            run_git(&["-C", clone_path_str, "checkout", "."])?;
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

    // clone_repository
    TmcCommand::new("git".to_string())
        .with(|e| {
            e.args(&["clone", "-q", "-b"])
                .arg(course_git_branch)
                .arg(course_source_url)
                .arg(new_clone_path)
                .stdout(Redirection::Pipe)
                .stderr(Redirection::Pipe)
        })
        .output_checked()?;
    Ok(())
}

/// Makes sure no directory directly under path is an exercise directory containing a dash in the relative path from path to the dir.
/// A dash is used as a special delimiter.
fn check_directory_names(path: &Path) -> Result<(), UtilError> {
    // exercise directories in canonicalized form
    for exercise_dir in task_executor::find_exercise_directories(path)? {
        let relative = exercise_dir.strip_prefix(path).unwrap();
        if relative.to_string_lossy().contains('-') {
            return Err(UtilError::InvalidDirectory(exercise_dir));
        }
    }
    Ok(())
}

/// Checks for a course_clone_path/course_options.yml
/// If found, course-specific options are merged into it and it is returned.
/// Else, an empty mapping is returned.
fn get_course_options(course_clone_path: &Path, course_name: &str) -> Result<Mapping, UtilError> {
    let options_file = course_clone_path.join("course_options.yml");
    if options_file.exists() {
        let file = file_util::open_file(options_file)?;
        let mut course_options: Mapping = serde_yaml::from_reader(file).unwrap();
        // try to remove the "courses" map
        if let Some(serde_yaml::Value::Mapping(mut courses)) =
            course_options.remove(&serde_yaml::Value::String("courses".to_string()))
        {
            // try to remove the map corresponding to the current course from the "courses" map
            if let Some(serde_yaml::Value::Mapping(mapping)) =
                courses.remove(&serde_yaml::Value::String(course_name.to_string()))
            {
                // if found, merge the inner course map with the base map
                for (key, value) in mapping {
                    course_options.insert(key, value);
                }
            }
        }
        Ok(course_options)
    } else {
        Ok(Mapping::new())
    }
}

/// Finds exercise directories, and converts the directories to "exercise names" by swapping the separators for dashes.
/// Also calculates checksums and fetches points for all
fn get_exercises(
    exercise_dirs: Vec<PathBuf>,
    course_clone_path: &Path,
    course_stub_path: &Path,
) -> Result<Vec<RefreshExercise>, UtilError> {
    let exercises = exercise_dirs
        .into_iter()
        .map(|exercise_dir| {
            log::debug!(
                "processing points and checksum for {}",
                exercise_dir.display()
            );
            let name = exercise_dir.to_string_lossy().replace("/", "-");

            // checksum
            let checksum = calculate_checksum(&course_stub_path.join(&exercise_dir))?;

            let exercise_path = course_clone_path.join(&exercise_dir);
            let points = task_executor::get_available_points(&exercise_path)?;

            Ok(RefreshExercise {
                name,
                points,
                checksum,
                path: exercise_dir,
            })
        })
        .collect::<Result<_, UtilError>>()?;
    Ok(exercises)
}

fn calculate_checksum(exercise_dir: &Path) -> Result<String, UtilError> {
    let mut digest = Context::new();

    // order filenames for a consistent hash
    for entry in WalkDir::new(exercise_dir)
        .min_depth(1) // do not hash the directory itself ('.')
        .sort_by(|a, b| a.file_name().cmp(b.file_name()))
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(exercise_dir).unwrap();
        let string = relative.as_os_str().to_string_lossy();
        log::debug!("updating {}", string);
        digest.consume(string.as_ref());
        if entry.path().is_file() {
            log::debug!("updating with file");
            let file = file_util::read_file(entry.path())?;
            digest.consume(file);
        }
    }

    // convert the digest into a hex string
    let digest = digest.compute();
    Ok(format!("{:x}", digest))
}

fn execute_zip(
    course_exercises: &[RefreshExercise],
    root_path: &Path,
    zip_dir: &Path,
) -> Result<(), UtilError> {
    file_util::create_dir_all(zip_dir)?;
    for exercise in course_exercises {
        let exercise_root = root_path.join(&exercise.path);
        let zip_file_path = zip_dir.join(format!("{}.zip", exercise.name));

        let mut writer = zip::ZipWriter::new(file_util::create_file(zip_file_path)?);
        for entry in WalkDir::new(&exercise_root).into_iter().filter_entry(|e| {
            !e.file_name()
                .to_str()
                .map(|e| e.starts_with('.'))
                .unwrap_or_default()
        })
        // filter hidden
        {
            let entry = entry?;
            if entry.path().is_file() {
                let relative_path = entry.path().strip_prefix(&root_path).unwrap(); // safe
                writer
                    .start_file(
                        relative_path.to_string_lossy(),
                        zip::write::FileOptions::default(),
                    )
                    .unwrap();
                let bytes = file_util::read_file(entry.path())?;
                writer.write_all(&bytes).map_err(UtilError::ZipWrite)?;
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(path: &Path) -> Result<(), UtilError> {
    // NOP on non-Unix platforms
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path) -> Result<(), UtilError> {
    use nix::sys::stat;
    use std::os::unix::io::AsRawFd;

    let chmod: ModeBits = 0o775; // octal, read and execute permissions for all users
    for entry in WalkDir::new(path) {
        let entry = entry?;
        let file = file_util::open_file(entry.path())?;
        stat::fchmod(
            file.as_raw_fd(),
            stat::Mode::from_bits(chmod).ok_or(UtilError::NixFlag(chmod))?,
        )
        .map_err(|e| UtilError::NixPermissionChange(path.to_path_buf(), e))?;
    }

    Ok(())
}

#[cfg(test)]
#[cfg(unix)] // not used on windows
mod test {
    use std::io::Read;

    use super::*;
    use serde_yaml::Value;
    use task_executor::find_exercise_directories;
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

    /*
    #[test]
    #[ignore = "uses git"]
    fn updates_repository() {
        init();

        let cache = tempfile::TempDir::new().unwrap();
        file_util::create_dir_all(cache.path().join("clone")).unwrap();
        let run_git = |args: &[&str], cwd: &Path| {
            TmcCommand::new("git")
                .with(|e| {
                    e.args(args)
                        .cwd(cwd)
                        .stdout(Redirection::Pipe)
                        .stderr(Redirection::Pipe)
                })
                .output_checked()
                .unwrap()
        };
        run_git(&["init"], &cache.path().join("clone"));
        assert!(cache.path().join("clone/.git").exists());

        let clone = tempfile::TempDir::new().unwrap();
        run_git(&["init"], &clone.path());
        run_git(&["remote", "add", "origin", ""], &clone.path());

        update_or_clone_repository(clone.path(), Path::new(GIT_REPO), "master", cache.path())
            .unwrap();
        assert!(clone.path().join("texts").exists());
    }

    #[test]
    #[ignore = "uses git"]
    fn clones_repository() {
        init();

        let clone = tempfile::TempDir::new().unwrap();
        assert!(!clone.path().join(".git").exists());
        let old_cache_path = Path::new("nonexistent");

        update_or_clone_repository(clone.path(), Path::new(GIT_REPO), "master", old_cache_path)
            .unwrap();
        assert!(clone.path().join("texts").exists());
    }
    */

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
    fn updates_course_options() {
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
    fn updates_course_options_merged() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "course_options.yml",
            r#"
courses:
    course_1:
        option: true
    course_2:
        other_option: true
"#,
        );
        let options = get_course_options(temp.path(), "course_1").unwrap();
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
        let exercise_dirs = find_exercise_directories(&temp.path().join("course")).unwrap();
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
        assert_eq!(exercises[0].checksum, "043fb4832da4e3fbf5babd13ed9fa732");
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
        file_to(&temp, "clone/part2/ex2/dir/subdir/.hidden", "");
        file_to(&temp, "stub/part1/ex1/setup.py", "");
        file_to(&temp, "stub/part1/ex2/setup.py", "");
        file_to(&temp, "stub/part2/ex1/setup.py", "");
        file_to(&temp, "stub/part2/ex2/setup.py", "");
        file_to(&temp, "stub/part2/ex2/dir/subdir/file", "some file");
        file_to(&temp, "stub/part2/ex2/dir/subdir/.hidden", "hidden file");

        let exercise_dirs = find_exercise_directories(&temp.path().join("clone")).unwrap();
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
        assert!(fz.by_name("setup.py").is_ok());
        assert!(fz
            .by_name(
                &Path::new("dir")
                    .join("subdir")
                    .join(".hidden")
                    .to_string_lossy()
            )
            .is_err());
        let mut file = fz
            .by_name(
                &Path::new("dir")
                    .join("subdir")
                    .join("file")
                    .to_string_lossy(),
            )
            .unwrap();
        let mut buf = String::new();
        file.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "some file");
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

        let checksum =
            calculate_checksum(Path::new("tests/data/course_refresher/valid_exercises/ex1"))
                .unwrap();
        assert_eq!(checksum, "6cacf02f21f9242674a876954132fb11");
    }
}
