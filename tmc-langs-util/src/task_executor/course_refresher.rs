use crate::{error::UtilError, task_executor};
use serde_yaml::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tmc_langs_framework::{command::TmcCommand, io::file_util};
use walkdir::WalkDir;

#[derive(Debug, PartialEq, Eq)]
pub enum SourceBackend {
    Git,
}

pub struct Point {
    name: String,
    requires_review: bool,
}

pub struct RefreshExercise {
    name: String,
    relative_path: PathBuf,
    has_tests: bool,
    available_points: Vec<String>,
}

pub struct Course {
    id: usize,
    name: String,
    cache_path: PathBuf,
    clone_path: PathBuf,
    stub_path: PathBuf,
    stub_zip_path: PathBuf,
    solution_path: PathBuf,
    solution_zip_path: PathBuf,
    exercises: Vec<RefreshExercise>,
    source_backend: SourceBackend,
    source_url: String,
    git_branch: String,
}

pub struct Options {
    no_directory_changes: bool,
    no_background_operations: bool,
}

pub fn refresh_course(
    mut course: Course,
    options: Options,
    git_repos_chmod: Option<String>,
    git_repos_chgrp: Option<String>,
    cache_root: PathBuf,
    rails_root: PathBuf,
) -> Result<(), UtilError> {
    let old_cache_path = &course.cache_path;

    // increment_cached_version not implemented

    if !options.no_directory_changes {
        file_util::remove_dir_all(&course.cache_path)?;
        file_util::create_dir_all(&course.cache_path)?;
    }

    if !options.no_directory_changes {
        // update_or_clone_repository
        if course.source_backend != SourceBackend::Git {
            log::error!("Source types other than git not yet implemented");
            return Err(UtilError::UnsupportedSourceBackend);
        }
        if old_cache_path.join("clone").join(".git").exists() {
            // Try a fast path: copy old clone and git fetch new stuff
            if copy_and_update_repository(
                old_cache_path,
                &course.clone_path,
                &course.source_url,
                &course.git_branch,
            )
            .is_err()
            {
                file_util::remove_dir_all(&course.clone_path)?;
                // clone_repository
                TmcCommand::new("git".to_string())
                    .with(|e| {
                        e.args(&["clone", "-q", "-b"])
                            .arg(&course.git_branch)
                            .arg(&course.source_url)
                            .arg(&course.clone_path)
                    })
                    .output_checked()?;
            }
        } else {
            // clone_repository
            TmcCommand::new("git".to_string())
                .with(|e| {
                    e.args(&["clone", "-q", "-b"])
                        .arg(&course.git_branch)
                        .arg(&course.source_url)
                        .arg(&course.clone_path)
                })
                .output_checked()?;
        }

        // check_directory_names
        let exercise_dirs = {
            let mut canon_dirs = vec![];
            for path in task_executor::find_exercise_directories(&course.clone_path) {
                let canon = path
                    .canonicalize()
                    .map_err(|e| UtilError::Canonicalize(path, e))?;
                canon_dirs.push(canon);
            }
            canon_dirs
        };
        for entry in WalkDir::new(&course.clone_path).min_depth(1) {
            let entry = entry?;
            let path = entry.path();
            let relpath = path.strip_prefix(&course.clone_path).unwrap();
            if path.is_dir()
                && exercise_dirs
                    .iter()
                    .any(|exdir| exdir.starts_with(path) && relpath.to_string_lossy().contains('-'))
            {
                return Err(UtilError::InvalidDirectory(path.to_path_buf()));
            }
        }
    }

    // update_course_options
    let merge_course_specific_options = |opts: &mut Mapping| {
        // try to remove the "courses" map
        if let Some(serde_yaml::Value::Mapping(mut courses)) =
            opts.remove(&serde_yaml::Value::String("courses".to_string()))
        {
            // try to remove the map corresponding to the current course from the "courses" map
            if let Some(serde_yaml::Value::Mapping(mapping)) =
                courses.remove(&serde_yaml::Value::String(course.name.clone()))
            {
                // if found, merge the inner course map with the base map
                for (key, value) in mapping {
                    opts.insert(key, value);
                }
            }
        }
    };

    let options_file = course.clone_path.join("course_options.yml");
    let _opts = if options_file.exists() {
        let file = file_util::open_file(options_file)?;
        let mut course_options: Mapping = serde_yaml::from_reader(file).unwrap();
        merge_course_specific_options(&mut course_options);
        course_options
    } else {
        Mapping::new()
    };

    // add_records_for_new_exercises
    let exercise_dirs = task_executor::find_exercise_directories(&course.clone_path);
    let exercise_names = exercise_dirs
        .into_iter()
        .map(|ed| {
            ed.strip_prefix(&course.clone_path)
                .unwrap_or(&ed)
                .to_string_lossy()
                .replace("/", "-")
        })
        .collect::<Vec<_>>();

    let mut new_exercises = vec![];
    for exercise_name in &exercise_names {
        if course.exercises.iter().any(|e| &e.name == exercise_name) {
            log::info!("Added new exercise {}", exercise_name);
            new_exercises.push(exercise_name.clone());
        }
    }

    // delete_records_for_removed_exercises
    let mut removed_exercises = vec![];
    for exercise in &course.exercises {
        if !exercise_names.contains(&exercise.name) {
            log::info!("Removed exercise {}", exercise.name);
            removed_exercises.push(exercise.name.clone());
        }
    }

    // update_exercise_options
    let exercise_default_options = {
        use Value::{Bool, Null, String};
        let mut defaults = Mapping::new();
        defaults.insert(String("deadline".to_string()), Null);
        defaults.insert(String("soft_deadline".to_string()), Null);
        defaults.insert(String("publish_time".to_string()), Null);
        defaults.insert(String("gdocs_sheet".to_string()), Null);
        defaults.insert(String("points_visible".to_string()), Bool(true));
        defaults.insert(String("hidden".to_string()), Bool(false));
        defaults.insert(String("returnable".to_string()), Null);
        defaults.insert(String("solution_visible_after".to_string()), Null);
        defaults.insert(
            String("valgrind_strategy".to_string()),
            String("fail".to_string()),
        );
        defaults.insert(String("runtime_params".to_string()), Null);
        defaults.insert(
            String("code_review_requests_enabled".to_string()),
            Bool(true),
        );
        defaults.insert(
            String("run_tests_locally_action_enabled".to_string()),
            Bool(true),
        );
        defaults
    };

    let mut review_points = HashMap::new();
    let mut metadata_map = HashMap::new();
    for exercise in &course.exercises {
        let root_dir = &course.clone_path;
        let target_dir = course.clone_path.join(&exercise.relative_path);
        let file_name = "metadata.yml";
        let defaults = exercise_default_options.clone();
        let file_preprocessor = |opts: &mut Mapping| merge_course_specific_options(opts);

        let subdirs = target_dir.strip_prefix(root_dir).unwrap().components(); // safe unwrap due to check above
        let mut result = defaults;

        let mut merge_file = |path: &Path| -> Result<(), UtilError> {
            if !path.exists() {
                return Ok(());
            }
            let file = file_util::open_file(path)?;
            if let Ok(mut yaml) = serde_yaml::from_reader::<_, Mapping>(file) {
                file_preprocessor(&mut yaml);
                for (key, value) in yaml {
                    result.insert(key, value);
                }
                todo!("deep merge");
            }
            Ok(())
        };

        merge_file(&root_dir.join(file_name))?;
        let mut rel_path = PathBuf::from(".");
        // traverse
        for component in subdirs {
            merge_file(&root_dir.join(&rel_path))?;
            rel_path = rel_path.join(component);
        }

        let metadata = result;
        let exercise_review_points = match metadata.get(&Value::String("review_points".to_string()))
        {
            Some(Value::Null) => vec![],
            Some(Value::String(string)) => {
                string.split_whitespace().map(|s| s.to_string()).collect()
            }
            Some(Value::Sequence(seq)) => seq
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect(),
            _ => vec![], // todo: what to do with other values?
        };
        review_points.insert(exercise.name.clone(), exercise_review_points);
        metadata_map.insert(exercise.name.clone(), metadata);
    }

    // set_has_tests_flags not implemented here

    let mut update_points = HashMap::new();
    if !options.no_background_operations {
        // update_available_points
        // scans the exercise directory and compares the points found (and review points) with what was given in the arguments
        // to find new and removed points
        for exercise in &mut course.exercises {
            let review_points = review_points.get(&exercise.name).unwrap(); // safe
            let mut point_names = HashSet::new();
            // let clone_path = course.clone_path.join(&exercise.relative_path); // unused

            let points_data = {
                if options.no_directory_changes {
                    todo!("?");
                }
                let path = course.clone_path.join(&exercise.relative_path);
                let exercise_name = exercise.name.clone();
                task_executor::scan_exercise(&path, exercise_name)?.tests
            };
            point_names.extend(points_data.into_iter().map(|p| p.points).flatten());
            point_names.extend(review_points.clone());

            let mut added = vec![];
            let mut removed = vec![];

            for name in &point_names {
                if !exercise.available_points.contains(name) {
                    added.push(name.clone());
                }
            }

            for point in &exercise.available_points {
                if !point_names.contains(point) {
                    removed.push(point.clone());
                }
            }

            if !added.is_empty() {
                log::info!(
                    "Added points to exercise {}: {}",
                    exercise.name,
                    added.join(" ")
                );
            }
            if !removed.is_empty() {
                log::info!(
                    "Removed points from exercise {}: {}",
                    exercise.name,
                    removed.join(" ")
                );
            }
            update_points.insert(exercise.name.clone(), (added, removed));
        }
    }

    if !options.no_directory_changes {
        // make_solutions
        task_executor::prepare_solutions(&[course.clone_path.clone()], &course.solution_path)?;
        // make_stubs
        let exercise_dirs = task_executor::find_exercise_directories(&course.clone_path);
        task_executor::prepare_stubs(exercise_dirs, &course.clone_path, &course.stub_path)?;
    }

    // checksum_stubs
    // { exercise_name: checksum of file paths and file contents }
    let mut checksum_stubs = HashMap::new();
    for e in &course.exercises {
        let mut digest = md5::Context::new();
        for entry in WalkDir::new(course.stub_path.join(&e.relative_path))
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
        {
            let entry = entry?;
            if entry.path().is_file() {
                digest.consume(entry.path().as_os_str().to_string_lossy().into_owned());
                let file = file_util::read_file(entry.path())?;
                digest.consume(file);
            }
        }
        checksum_stubs.insert(e.name.clone(), digest.compute());
    }

    if !options.no_directory_changes {
        let execute_zip = |root_path: &Path, zip_path: &Path| -> Result<(), UtilError> {
            file_util::create_dir_all(zip_path)?;
            for e in &course.exercises {
                let mut stdin = String::new();
                let root = root_path.join(&e.relative_path);
                for entry in WalkDir::new(&root)
                    .min_depth(1)
                    .sort_by(|a, b| a.file_name().cmp(b.file_name()))
                {
                    let entry = entry?;
                    let stub_path = entry.path().strip_prefix(&root).unwrap(); // safe
                    stdin.push_str(&format!("{}\n", e.relative_path.join(stub_path).display()));
                }
                let zip_file_path = zip_path.join(format!("{}.zip", e.name));
                TmcCommand::new("zip")
                    .with(|e| {
                        e.arg("--quiet")
                            .arg("-@")
                            .arg(zip_file_path)
                            .cwd(&root_path)
                            .stdin(stdin.as_str())
                    })
                    .output_checked()?;
            }
            Ok(())
        };
        // make_zips_of_stubs
        execute_zip(&course.stub_path, &course.stub_zip_path)?;

        // make_zips_of_solutions
        execute_zip(&course.solution_path, &course.solution_zip_path)?;

        // set_permissions
        let chmod = git_repos_chmod;
        let chgrp = git_repos_chgrp;

        // check directories from rails root to cache root
        let cache_relative = cache_root.strip_prefix(&rails_root).unwrap(); // todo: handle err
        let components = cache_relative.components();
        let mut next = rails_root.clone();

        let run_chmod = |dir: &Path| -> Result<(), UtilError> {
            if let Some(chmod) = &chmod {
                TmcCommand::new("chmod")
                    .with(|e| e.arg(chmod).cwd(dir))
                    .output_checked()?;
            }
            Ok(())
        };
        let run_chgrp = |dir: &Path| -> Result<(), UtilError> {
            if let Some(chgrp) = &chgrp {
                TmcCommand::new("chgrp")
                    .with(|e| e.arg(chgrp).cwd(dir))
                    .output_checked()?;
            }
            Ok(())
        };
        run_chmod(&next)?;
        run_chgrp(&next)?;
        for component in components {
            next.push(component);
            run_chmod(&next)?;
            run_chgrp(&next)?;
        }
        if let Some(chmod) = &chmod {
            TmcCommand::new("chmod")
                .with(|e| e.arg("-R").arg(chmod).cwd(&course.cache_path))
                .output_checked()?;
        }
        if let Some(chgrp) = &chgrp {
            TmcCommand::new("chgrp")
                .with(|e| e.arg("-R").arg(chgrp).cwd(&course.cache_path))
                .output_checked()?;
        }
    }

    // invalidate_unlocks not implemented
    // kafka_publish_exercises not implemented

    Ok(())
}

fn copy_and_update_repository(
    old_cache_path: &Path,
    clone_path: &Path,
    source_url: &str,
    git_branch: &str,
) -> Result<(), UtilError> {
    let old_clone_path = old_cache_path.join("clone");
    file_util::copy(old_clone_path, clone_path)?;

    let run_git = |args: &[&str]| {
        TmcCommand::new("git".to_string())
            .with(|e| {
                e.cwd(clone_path)
                    .arg("-C")
                    .arg(clone_path.as_os_str())
                    .args(args)
            })
            .output_checked()
    };

    run_git(&["remote", "set-url", "origin", &source_url])?;
    run_git(&["fetch", "origin"])?;
    run_git(&["checkout", &format!("origin/{}", git_branch)])?;
    run_git(&["clean", "-df"])?;
    run_git(&["checkout", "."])?;
    Ok(())
}
