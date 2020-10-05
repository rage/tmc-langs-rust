use crate::{error::UtilError, task_executor};
use md5::{Context, Digest};
use serde_yaml::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tmc_langs_framework::{command::TmcCommand, io::file_util};
use walkdir::WalkDir;

#[derive(Debug, PartialEq, Eq)]
pub enum SourceBackend {
    Git,
}

pub struct RefreshExercise {
    name: String,
    relative_path: PathBuf,
    available_points: Vec<String>,
}

pub struct Course {
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

pub struct RefreshData {
    pub new_exercises: Vec<String>,
    pub removed_exercises: Vec<String>,
    pub review_points: HashMap<String, Vec<String>>,
    pub metadata: HashMap<String, Mapping>,
    pub checksum_stubs: HashMap<String, Digest>,
}

pub fn refresh_course(
    course: Course,
    options: Options,
    git_repos_chmod: Option<String>,
    git_repos_chgrp: Option<String>,
    cache_root: PathBuf,
    rails_root: PathBuf,
) -> Result<RefreshData, UtilError> {
    let old_cache_path = &course.cache_path;

    // increment_cached_version not implemented

    if !options.no_directory_changes {
        file_util::remove_dir_all(&course.cache_path)?;
        file_util::create_dir_all(&course.cache_path)?;
    }

    if !options.no_directory_changes {
        update_or_clone_repository(&course, &old_cache_path)?;
        check_directory_names(&course)?;
    }

    update_course_options(&course)?;

    // add_records_for_new_exercises & delete_records_for_removed_exercises
    let (new_exercises, removed_exercises) = update_exercises(&course);
    let ExerciseOptions {
        review_points,
        metadata_map: metadata,
    } = update_exercise_options(&course)?;

    // set_has_tests_flags not implemented here

    let mut update_points = HashMap::new();
    if !options.no_background_operations {
        update_available_points(&course, &options, &review_points, &mut update_points)?;
    }

    if !options.no_directory_changes {
        // make_solutions
        task_executor::prepare_solutions(&[course.clone_path.clone()], &course.solution_path)?;
        // make_stubs
        let exercise_dirs = task_executor::find_exercise_directories(&course.clone_path);
        task_executor::prepare_stubs(exercise_dirs, &course.clone_path, &course.stub_path)?;
    }

    let checksum_stubs = checksum_stubs(&course)?;

    if !options.no_directory_changes {
        // make_zips_of_stubs
        execute_zip(&course, &course.stub_path, &course.stub_zip_path)?;

        // make_zips_of_solutions
        execute_zip(&course, &course.solution_path, &course.solution_zip_path)?;

        // set_permissions
        set_permissions(
            &course,
            git_repos_chmod,
            git_repos_chgrp,
            &cache_root,
            rails_root,
        )?;
    }

    // invalidate_unlocks not implemented
    // kafka_publish_exercises not implemented

    Ok(RefreshData {
        new_exercises,
        removed_exercises,
        review_points,
        metadata,
        checksum_stubs,
    })
}

fn update_or_clone_repository(course: &Course, old_cache_path: &Path) -> Result<(), UtilError> {
    if course.source_backend != SourceBackend::Git {
        log::error!("Source types other than git not yet implemented");
        return Err(UtilError::UnsupportedSourceBackend);
    }
    if old_cache_path.join("clone").join(".git").exists() {
        // Try a fast path: copy old clone and git fetch new stuff
        let copy_and_update_repository = || -> Result<(), UtilError> {
            let old_clone_path = old_cache_path.join("clone");
            file_util::copy(old_clone_path, &course.clone_path)?;

            let run_git = |args: &[&str]| {
                TmcCommand::new("git".to_string())
                    .with(|e| {
                        e.cwd(&course.clone_path)
                            .arg("-C")
                            .arg(&course.clone_path.as_os_str())
                            .args(args)
                    })
                    .output_checked()
            };

            run_git(&["remote", "set-url", "origin", &course.source_url])?;
            run_git(&["fetch", "origin"])?;
            run_git(&["checkout", &format!("origin/{}", &course.git_branch)])?;
            run_git(&["clean", "-df"])?;
            run_git(&["checkout", "."])?;
            Ok(())
        };
        if copy_and_update_repository().is_err() {
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
    Ok(())
}

fn check_directory_names(course: &Course) -> Result<(), UtilError> {
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
    Ok(())
}

fn update_course_options(course: &Course) -> Result<(), UtilError> {
    let options_file = course.clone_path.join("course_options.yml");
    let _opts = if options_file.exists() {
        let file = file_util::open_file(options_file)?;
        let mut course_options: Mapping = serde_yaml::from_reader(file).unwrap();
        merge_course_specific_options(course, &mut course_options);
        course_options
    } else {
        Mapping::new()
    };
    Ok(())
}

fn merge_course_specific_options(course: &Course, opts: &mut Mapping) {
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
}

fn update_exercises(course: &Course) -> (Vec<String>, Vec<String>) {
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

    let mut removed_exercises = vec![];
    for exercise in &course.exercises {
        if !exercise_names.contains(&exercise.name) {
            log::info!("Removed exercise {}", exercise.name);
            removed_exercises.push(exercise.name.clone());
        }
    }
    (new_exercises, removed_exercises)
}

struct ExerciseOptions {
    review_points: HashMap<String, Vec<String>>,
    metadata_map: HashMap<String, Mapping>,
}

fn update_exercise_options(course: &Course) -> Result<ExerciseOptions, UtilError> {
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
        let file_preprocessor = |opts: &mut Mapping| merge_course_specific_options(&course, opts);

        let subdirs = target_dir.strip_prefix(root_dir).unwrap().components(); // safe unwrap due to check above
        let mut result = defaults;

        let mut merge_file = |path: &Path| -> Result<(), UtilError> {
            if !path.exists() {
                return Ok(());
            }
            let file = file_util::open_file(path)?;
            if let Ok(mut yaml) = serde_yaml::from_reader::<_, Mapping>(file) {
                file_preprocessor(&mut yaml);
                deep_merge_mappings(yaml, &mut result);
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

    Ok(ExerciseOptions {
        review_points,
        metadata_map,
    })
}

fn update_available_points(
    course: &Course,
    options: &Options,
    review_points: &HashMap<String, Vec<String>>,
    update_points: &mut HashMap<String, (Vec<String>, Vec<String>)>,
) -> Result<(), UtilError> {
    // scans the exercise directory and compares the points found (and review points) with what was given in the arguments
    // to find new and removed points
    for exercise in &course.exercises {
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
    Ok(())
}

fn checksum_stubs(course: &Course) -> Result<HashMap<String, Digest>, UtilError> {
    let mut checksum_stubs = HashMap::new();
    for e in &course.exercises {
        let mut digest = Context::new();
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
    Ok(checksum_stubs)
}

fn execute_zip(course: &Course, root_path: &Path, zip_path: &Path) -> Result<(), UtilError> {
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
}

fn set_permissions(
    course: &Course,
    chmod: Option<String>,
    chgrp: Option<String>,
    cache_root: &Path,
    rails_root: PathBuf,
) -> Result<(), UtilError> {
    // check directories from rails root to cache root
    let cache_relative = cache_root.strip_prefix(&rails_root).map_err(|e| {
        UtilError::CacheNotInRailsRoot(cache_root.to_path_buf(), rails_root.clone(), e)
    })?;
    let components = cache_relative.components();
    let mut next = rails_root;

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
    Ok(())
}

fn deep_merge_mappings(from: Mapping, into: &mut Mapping) {
    for (key, value) in from {
        if let Value::Mapping(from_mapping) = value {
            if let Some(Value::Mapping(into_mapping)) = into.get_mut(&key) {
                // combine mappings
                deep_merge_mappings(from_mapping, into_mapping);
            } else {
                into.insert(key, Value::Mapping(from_mapping));
            }
        } else {
            into.insert(key, value);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn updates_repository() {
        init();

        let course = Course {
            name: "name".to_string(),
            cache_path: PathBuf::from(""),
            clone_path: PathBuf::from(""),
            stub_path: PathBuf::from(""),
            stub_zip_path: PathBuf::from(""),
            solution_path: PathBuf::from(""),
            solution_zip_path: PathBuf::from(""),
            exercises: vec![],
            source_backend: SourceBackend::Git,
            source_url: "".to_string(),
            git_branch: "".to_string(),
        };
        let old_cache_path = Path::new("");
        update_or_clone_repository(&course, old_cache_path).unwrap();
    }
}
