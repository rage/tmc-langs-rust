#![deny(clippy::print_stdout, clippy::print_stderr)]

//! The main tmc-langs library.

pub mod config;
pub mod course_refresher;
pub mod data;
mod error;
mod submission_packaging;
mod submission_processing;

use config::Credentials;
pub use tmc_client::{oauth2, ClientUpdateData};
pub use tmc_client::{ClientError, FeedbackAnswer, TmcClient, Token};
pub use tmc_client::{
    Course, CourseData, CourseDetails, CourseExercise, ExerciseDetails, NewSubmission,
    Organization, Review, RunResult, StyleValidationResult, Submission, SubmissionFeedbackResponse,
    SubmissionFinished, UpdateResult,
};
pub use tmc_langs_framework::{
    CommandError, ExerciseDesc, ExercisePackagingConfiguration, Language, LanguagePlugin,
};
pub use tmc_langs_util::{
    file_util::{self, FileLockGuard},
    warning_reporter,
};

use crate::config::{ProjectsConfig, TmcConfig};
use crate::data::LocalExercise;
use crate::error::LangsError;

pub use crate::course_refresher::refresh_course;
pub use crate::submission_packaging::prepare_submission;
pub use crate::submission_processing::prepare_solution;

use heim::disk;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tmc_langs_framework::{NothingIsStudentFilePolicy, StudentFilePolicy, TmcError, TmcProjectYml};
use tmc_langs_plugins::tmc_zip;
use tmc_langs_util::progress_reporter;
use walkdir::WalkDir;

pub fn init_tmc_client_with_credentials(
    root_url: String,
    client_name: &str,
    client_version: &str,
) -> Result<TmcClient, LangsError> {
    // create client
    let mut client = TmcClient::new_in_config(
        root_url,
        client_name.to_string(),
        client_version.to_string(),
    )?;

    // set token from the credentials file if one exists
    let credentials = Credentials::load(client_name)?;
    if let Some(credentials) = &credentials {
        client.set_token(credentials.token())?;
    }

    Ok(client)
}

pub fn checkstyle(
    exercise_path: &Path,
    locale: Language,
) -> Result<Option<StyleValidationResult>, LangsError> {
    let style_validation_result = tmc_langs_plugins::get_language_plugin(exercise_path)?
        .check_code_style(exercise_path, locale)?;
    Ok(style_validation_result)
}

pub fn clean(exercise_path: &Path) -> Result<(), LangsError> {
    tmc_langs_plugins::get_language_plugin(exercise_path)?.clean(exercise_path)?;
    Ok(())
}

pub fn compress_project_to(source: &Path, target: &Path) -> Result<(), LangsError> {
    let data = tmc_langs_plugins::compress_project(source)?;

    if let Some(parent) = target.parent() {
        file_util::create_dir_all(parent)?;
    }
    file_util::write_to_file(&data, target)?;
    Ok(())
}

pub fn free_disk_space_megabytes(path: &Path) -> Result<u64, LangsError> {
    let usage = smol::block_on(disk::usage(path))?
        .free()
        .get::<heim::units::information::megabyte>();
    Ok(usage)
}

pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    clean: bool,
) -> Result<(), LangsError> {
    if let Ok(plugin) = tmc_langs_plugins::get_language_plugin(target_location) {
        plugin.extract_project(compressed_project, target_location, clean)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, LangsError> {
    let points = tmc_langs_plugins::get_language_plugin(exercise_path)?
        .get_available_points(exercise_path)?;
    Ok(points)
}

pub fn find_exercise_directories(exercise_path: &Path) -> Result<Vec<PathBuf>, LangsError> {
    log::info!(
        "finding exercise directories in {}",
        exercise_path.display()
    );

    let mut paths = vec![];
    for entry in WalkDir::new(exercise_path).into_iter().filter_entry(|e| {
        !submission_processing::is_hidden_dir(e)
            && e.file_name() != "private"
            && !submission_processing::contains_tmcignore(e)
    }) {
        let entry = entry?;
        // check if the path contains a valid exercise for some plugin
        if tmc_langs_plugins::get_language_plugin(entry.path()).is_ok() {
            paths.push(entry.into_path())
        }
    }
    Ok(paths)
}

pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, LangsError> {
    let config = TmcProjectYml::from(path)?;
    Ok(tmc_langs_plugins::get_language_plugin(path)?
        .get_exercise_packaging_configuration(config)?)
}

pub fn list_local_course_exercises(
    client_name: &str,
    course_slug: &str,
) -> Result<Vec<LocalExercise>, LangsError> {
    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    let mut projects_config = ProjectsConfig::load(&projects_dir)?;

    let exercises = projects_config
        .courses
        .remove(course_slug)
        .map(|cc| cc.exercises)
        .unwrap_or_default();
    let mut local_exercises: Vec<LocalExercise> = vec![];
    for (exercise_slug, _) in exercises {
        local_exercises.push(LocalExercise {
            exercise_path: projects_dir.join(course_slug).join(&exercise_slug),
            exercise_slug,
        })
    }
    Ok(local_exercises)
}

pub fn prepare_stub(exercise_path: &Path, dest_path: &Path) -> Result<(), LangsError> {
    submission_processing::prepare_stub(&exercise_path, dest_path)?;

    // The Ant plugin needs some additional files to be copied over.
    if tmc_langs_plugins::AntPlugin::is_exercise_type_correct(&exercise_path) {
        tmc_langs_plugins::AntPlugin::copy_tmc_junit_runner(dest_path)
            .map_err(|e| TmcError::Plugin(Box::new(e)))?;
    }
    Ok(())
}

pub fn run_tests(path: &Path) -> Result<RunResult, LangsError> {
    Ok(tmc_langs_plugins::get_language_plugin(path)?.run_tests(path)?)
}

pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, LangsError> {
    Ok(tmc_langs_plugins::get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

pub fn extract_student_files(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), LangsError> {
    if let Ok(plugin) = tmc_langs_plugins::get_language_plugin(target_location) {
        plugin.extract_student_files(compressed_project, target_location)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

fn move_dir(source: &Path, source_lock: FileLockGuard, target: &Path) -> Result<(), LangsError> {
    let mut file_count_copied = 0;
    let mut file_count_total = 0;
    for entry in WalkDir::new(source) {
        let entry = entry?;
        if entry.path().is_file() {
            file_count_total += 1;
        }
    }
    start_stage(
        file_count_total + 1,
        format!("Moving dir {} -> {}", source.display(), target.display()),
    );

    for entry in WalkDir::new(source).contents_first(true).min_depth(1) {
        let entry = entry?;
        let entry_path = entry.path();

        if entry_path.file_name() == Some(OsStr::new(".tmc.lock")) {
            log::info!("skipping lock file");
            file_count_copied += 1;
            progress_stage(format!(
                "Skipped moving file {} / {}",
                file_count_copied, file_count_total
            ));
            continue;
        }

        if entry_path.is_file() {
            let relative = entry_path
                .strip_prefix(source)
                .expect("the entry is inside the source");
            let target_path = target.join(relative);
            log::debug!(
                "Moving {} -> {}",
                entry_path.display(),
                target_path.display()
            );

            // create parent dir for target and copy it, remove source file after
            if let Some(parent) = target_path.parent() {
                file_util::create_dir_all(parent)?;
            }
            file_util::copy(entry_path, &target_path)?;
            file_util::remove_file(entry_path)?;

            file_count_copied += 1;
            progress_stage(format!(
                "Moved file {} / {}",
                file_count_copied, file_count_total
            ));
        } else if entry_path.is_dir() {
            log::debug!("Deleting {}", entry_path.display());
            file_util::remove_dir_empty(entry_path)?;
        }
    }

    drop(source_lock);
    file_util::remove_dir_empty(source)?;

    finish_stage("Finished moving project directory");
    Ok(())
}

fn start_stage(steps: usize, message: impl Into<String>) {
    progress_reporter::start_stage::<()>(steps, message.into(), None)
}

fn progress_stage(message: impl Into<String>) {
    progress_reporter::progress_stage::<()>(message.into(), None)
}

fn finish_stage(message: impl Into<String>) {
    progress_reporter::finish_stage::<()>(message.into(), None)
}

fn extract_project_overwrite(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), LangsError> {
    tmc_zip::unzip(
        NothingIsStudentFilePolicy::new(target_location)?,
        compressed_project,
        target_location,
    )?;
    Ok(())
}
