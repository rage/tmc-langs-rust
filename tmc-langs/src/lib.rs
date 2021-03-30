//! The main tmc-langs library. Provides a convenient API to all of the functionality provided by the tmc-langs project.

#![deny(clippy::print_stdout, clippy::print_stderr)]

mod config;
mod course_refresher;
mod data;
mod error;
mod submission_packaging;
mod submission_processing;

pub use crate::config::{
    list_local_course_exercises, migrate_exercise, move_projects_dir, ConfigValue, CourseConfig,
    Credentials, ProjectsDirExercise, TmcConfig,
};
pub use crate::course_refresher::{refresh_course, RefreshData, RefreshExercise};
pub use crate::data::{
    CombinedCourseData, DownloadOrUpdateCourseExercisesResult, DownloadResult, ExerciseDownload,
    LocalExercise, OutputFormat, TmcParams,
};
pub use crate::error::{LangsError, ParamError};
pub use crate::submission_packaging::prepare_submission;
pub use crate::submission_processing::prepare_solution;
pub use tmc_client::{
    ClientError, ClientUpdateData, Course, CourseData, CourseDetails, CourseExercise,
    ExerciseDetails, FeedbackAnswer, NewSubmission, Organization, Review, RunResult,
    StyleValidationResult, Submission, SubmissionFeedbackResponse, SubmissionFinished, TmcClient,
    Token, UpdateResult,
};
pub use tmc_langs_framework::{
    CommandError, ExerciseDesc, ExercisePackagingConfiguration, Language, LanguagePlugin,
};
pub use tmc_langs_util::{
    file_util::{self, FileLockGuard},
    warning_reporter,
};

use crate::config::ProjectsConfig;
use crate::data::DownloadTarget;
// use heim::disk;
use oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
use serde_json::Value as JsonValue;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use std::{collections::HashMap, ffi::OsStr};
use tmc_langs_framework::{NothingIsStudentFilePolicy, StudentFilePolicy, TmcError, TmcProjectYml};
use tmc_langs_plugins::{get_language_plugin, tmc_zip, AntPlugin, Plugin};
use tmc_langs_util::progress_reporter;
use toml::{map::Map as TomlMap, Value as TomlValue};
use url::Url;
use walkdir::WalkDir;

/// Returns the projects directory for the given client name.
pub fn get_projects_dir(client_name: &str) -> Result<PathBuf, LangsError> {
    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    Ok(projects_dir)
}

/// Checks the server for any updates for exercises within the given projects directory.
/// Returns the ids of each exercise that can be updated.
pub fn check_exercise_updates(
    client: &TmcClient,
    projects_dir: &Path,
) -> Result<Vec<usize>, LangsError> {
    log::debug!("checking exercise updates in {}", projects_dir.display());

    let mut updated_exercises = vec![];

    let config = ProjectsConfig::load(projects_dir)?;
    let local_exercises = config.get_all_exercises().collect::<Vec<_>>();

    // request would fail with empty id list
    if !local_exercises.is_empty() {
        let exercise_ids = local_exercises.iter().map(|e| e.id).collect::<Vec<_>>();
        let server_exercises = client
            .get_exercises_details(&exercise_ids)?
            .into_iter()
            .map(|e| (e.id, e))
            .collect::<HashMap<_, _>>();
        for local_exercise in local_exercises {
            let server_exercise = server_exercises
                .get(&local_exercise.id)
                .ok_or_else(|| LangsError::ExerciseMissingOnServer(local_exercise.id))?;
            if server_exercise.checksum != local_exercise.checksum {
                // server has an updated exercise
                updated_exercises.push(local_exercise.id);
            }
        }
    }
    Ok(updated_exercises)
}

/// Downloads the user's old submission from the server.
/// Resets the exercise at the path before the download.
/// If a submission_url is given, the current state of the exercise is submitted to that URL before the download.
pub fn download_old_submission(
    client: &TmcClient,
    exercise_id: usize,
    output_path: PathBuf,
    submission_id: usize,
    submission_url: Option<Url>,
) -> Result<(), LangsError> {
    log::debug!(
        "downloading old submission {} for {}",
        submission_id,
        exercise_id
    );

    if let Some(submission_url) = submission_url {
        // submit old exercise
        client.submit(submission_url, &output_path, None)?;
        log::debug!("finished submission");
    }

    // reset old exercise
    client.reset(exercise_id, output_path.clone())?;
    log::debug!("reset exercise");

    // dl submission
    let temp_zip = file_util::named_temp_file()?;
    client.download_old_submission(submission_id, temp_zip.path())?;
    log::debug!("downloaded old submission to {}", temp_zip.path().display());

    // extract submission
    extract_student_files(temp_zip, &output_path)?;
    log::debug!("extracted project");
    Ok(())
}

/// Downloads the given exercises, by either downloading the exercise template, updating the exercise or downloading an old submission.
/// If the exercise doesn't exist on disk yet...
///   if there are previous submissions and download_template is not set, the latest submission is downloaded.
///   otherwise, the exercise template is downloaded.
/// If the exercise exists on disk, it is updated using the course template.
pub fn download_or_update_course_exercises(
    client: &TmcClient,
    projects_dir: &Path,
    exercises: &[usize],
    download_template: bool,
) -> Result<DownloadResult, LangsError> {
    log::debug!(
        "downloading or updating course exercises in {}",
        projects_dir.display()
    );

    let exercises_details = client.get_exercises_details(exercises)?;

    let mut projects_config = ProjectsConfig::load(projects_dir)?;

    // separate exercises into downloads and skipped
    let mut to_be_downloaded = vec![];
    let mut to_be_skipped = vec![];

    log::debug!("checking the checksum of each exercise on the server");
    for exercise_detail in exercises_details {
        let target = ProjectsConfig::get_exercise_download_target(
            projects_dir,
            &exercise_detail.course_name,
            &exercise_detail.exercise_name,
        );

        // check if the exercise is already on disk
        if let Some(exercise) = projects_config
            .get_exercise(&exercise_detail.course_name, &exercise_detail.exercise_name)
        {
            // exercise is on disk, check if the checksum is identical
            if exercise_detail.checksum == exercise.checksum {
                // skip this exercise
                log::info!(
                    "Skipping exercise {} ({} in {}) due to identical checksum",
                    exercise_detail.id,
                    exercise_detail.course_name,
                    exercise_detail.exercise_name
                );
                to_be_skipped.push(ExerciseDownload {
                    id: exercise_detail.id,
                    course_slug: exercise_detail.course_name,
                    exercise_slug: exercise_detail.exercise_name,
                    path: target,
                });
                continue;
            }
        } else {
            // not on disk, if flag isn't set check if there are any previous submissions and take the latest one if so
            if !download_template {
                if let Some(latest_submission) = client
                    .get_exercise_submissions_for_current_user(exercise_detail.id)?
                    .into_iter()
                    .max_by_key(|s| s.created_at)
                {
                    // previous submission found
                    to_be_downloaded.push(DownloadTarget::Submission {
                        target: ExerciseDownload {
                            id: exercise_detail.id,
                            course_slug: exercise_detail.course_name,
                            exercise_slug: exercise_detail.exercise_name,
                            path: target,
                        },
                        submission_id: latest_submission.id,
                        checksum: exercise_detail.checksum,
                    });
                    continue;
                }
            }
        }

        // not skipped, either not on disk or no previous submissions, downloading template
        to_be_downloaded.push(DownloadTarget::Template {
            target: ExerciseDownload {
                id: exercise_detail.id,
                course_slug: exercise_detail.course_name.clone(),
                exercise_slug: exercise_detail.exercise_name.clone(),
                path: target,
            },
            checksum: exercise_detail.checksum,
        });
    }

    log::debug!("downloading exercises");
    // download and divide the results into successful and failed downloads
    let thread_count = to_be_downloaded.len().min(4); // max 4 threads
    let mut handles = vec![];
    let exercises = Arc::new(Mutex::new(to_be_downloaded));
    for _thread_id in 0..thread_count {
        let client = client.clone();
        let exercises = Arc::clone(&exercises);

        // each thread returns either a list of successful downloads, or a tuple of successful downloads and errors
        type ThreadErr = (Vec<DownloadTarget>, Vec<(DownloadTarget, LangsError)>);
        let handle = std::thread::spawn(move || -> Result<Vec<DownloadTarget>, ThreadErr> {
            let mut downloaded = vec![];
            let mut failed = vec![];

            // repeat until out of exercises
            loop {
                let mut exercises = exercises.lock().expect("the threads should never panic");
                let download_target = if let Some(download_target) = exercises.pop() {
                    download_target
                } else {
                    // no exercises left, break loop and exit thread
                    break;
                };
                drop(exercises);
                // dropped mutex

                let exercise_download_result = || -> Result<(), LangsError> {
                    let zip_file = file_util::named_temp_file()?;
                    match &download_target {
                        DownloadTarget::Template { target, .. } => {
                            client.download_exercise(target.id, zip_file.path())?;
                            extract_project(zip_file, &target.path, false)?;
                        }
                        DownloadTarget::Submission {
                            target,
                            submission_id,
                            ..
                        } => {
                            client.download_exercise(target.id, zip_file.path())?;
                            extract_project(&zip_file, &target.path, false)?;

                            let plugin = get_language_plugin(&target.path)?;
                            let tmc_project_yml = TmcProjectYml::from(&target.path)?;
                            let config =
                                plugin.get_exercise_packaging_configuration(tmc_project_yml)?;
                            for student_file in config.student_file_paths {
                                if student_file.is_file() {
                                    file_util::remove_file(&student_file)?;
                                }
                            }

                            client.download_old_submission(*submission_id, zip_file.path())?;
                            extract_project(&zip_file, &target.path, false)?;
                        }
                    }
                    Ok(())
                }();

                match exercise_download_result {
                    Ok(_) => {
                        downloaded.push(download_target);
                    }
                    Err(err) => {
                        failed.push((download_target, err));
                    }
                }
            }
            if failed.is_empty() {
                Ok(downloaded)
            } else {
                Err((downloaded, failed))
            }
        });
        handles.push(handle);
    }

    let mut successful = vec![];
    let mut failed = vec![];
    for handle in handles {
        match handle.join().expect("the threads should never panic") {
            Ok(s) => successful.extend(s),
            Err((s, f)) => {
                successful.extend(s);
                failed.extend(f);
            }
        }
    }

    log::debug!("save updated information to the course config");
    // turn the downloaded exercises into a hashmap with the course as key
    let mut course_data = HashMap::<String, Vec<(String, String, usize)>>::new();
    for download_target in &successful {
        let (target, checksum) = match download_target {
            DownloadTarget::Submission {
                target, checksum, ..
            }
            | DownloadTarget::Template {
                target, checksum, ..
            } => (target, checksum),
        };
        let entry = course_data.entry(target.course_slug.clone());
        let course_exercises = entry.or_default();
        course_exercises.push((target.exercise_slug.clone(), checksum.clone(), target.id));
    }
    // update/create the course configs that contain downloaded or updated exercises
    for (course_name, exercise_names) in course_data {
        let exercises = exercise_names
            .into_iter()
            .map(|(name, checksum, id)| (name, ProjectsDirExercise { id, checksum }))
            .collect();
        if let Some(course_config) = projects_config.courses.get_mut(&course_name) {
            course_config.exercises.extend(exercises);
            course_config.save_to_projects_dir(projects_dir)?;
        } else {
            let course_config = CourseConfig {
                course: course_name,
                exercises,
            };
            course_config.save_to_projects_dir(projects_dir)?;
        };
    }

    let downloaded = successful
        .into_iter()
        .map(|t| match t {
            DownloadTarget::Submission { target, .. } => target,
            DownloadTarget::Template { target, .. } => target,
        })
        .collect();

    // return an error if any downloads failed
    if !failed.is_empty() {
        // add an error trace to each failed download
        let failed = failed
            .into_iter()
            .map(|(target, err)| {
                let mut error = &err as &dyn std::error::Error;
                let mut chain = vec![error.to_string()];
                while let Some(source) = error.source() {
                    chain.push(source.to_string());
                    error = source;
                }
                let target = match target {
                    DownloadTarget::Submission { target, .. }
                    | DownloadTarget::Template { target, .. } => target,
                };
                (target, chain)
            })
            .collect();
        return Ok(DownloadResult::Failure {
            downloaded,
            skipped: to_be_skipped,
            failed,
        });
    }

    Ok(DownloadResult::Success {
        downloaded,
        skipped: to_be_skipped,
    })
}

/// Fetches the given course's details, exercises and course data.
pub fn get_course_data(
    client: &TmcClient,
    course_id: usize,
) -> Result<CombinedCourseData, LangsError> {
    log::debug!("getting course data for {}", course_id);

    let details = client.get_course_details(course_id)?;
    let exercises = client.get_course_exercises(course_id)?;
    let settings = client.get_course(course_id)?;
    Ok(CombinedCourseData {
        details,
        exercises,
        settings,
    })
}

/// Creates a login Token from a token string.
pub fn login_with_token(token: String) -> Token {
    log::debug!("creating token from token string");

    let mut token_response = StandardTokenResponse::new(
        AccessToken::new(token),
        BasicTokenType::Bearer,
        EmptyExtraTokenFields {},
    );
    token_response.set_scopes(Some(vec![Scope::new("public".to_string())]));
    token_response
}

/// Authenticates with the server, returning a login Token.
/// Reads the password from stdin.
pub fn login_with_password(
    client: &mut TmcClient,
    base64: bool,
    client_name: &str,
    email: String,
) -> Result<Token, LangsError> {
    log::debug!("logging in with password");

    // TODO: print "Please enter password" and add "quiet"  flag
    let password = rpassword::read_password().map_err(LangsError::ReadPassword)?;
    let decoded = if base64 {
        let bytes = base64::decode(password)?;
        String::from_utf8(bytes).map_err(LangsError::Base64PasswordNotUtf8)?
    } else {
        password
    };
    let token = client.authenticate(client_name, email.to_string(), decoded)?;
    Ok(token)
}

/// Initializes a TmcClient, using and returning the stored credentials, if any.
pub fn init_tmc_client_with_credentials(
    root_url: String,
    client_name: &str,
    client_version: &str,
) -> Result<(TmcClient, Option<Credentials>), LangsError> {
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

    Ok((client, credentials))
}

/// Updates the exercises in the local projects directory.
// TODO: parallel downloads
pub fn update_exercises(
    client: &TmcClient,
    client_name: &str,
) -> Result<DownloadOrUpdateCourseExercisesResult, LangsError> {
    log::debug!("updating exercises for {}", client_name);

    let mut exercises_to_update = vec![];
    let course_data = HashMap::<String, Vec<(String, String, usize)>>::new();

    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    let mut projects_config = ProjectsConfig::load(&projects_dir)?;
    let local_exercises = projects_config
        .courses
        .iter_mut()
        .map(|c| &mut c.1.exercises)
        .flatten()
        .map(|e| e.1)
        .collect::<Vec<_>>();
    let exercise_ids = local_exercises.iter().map(|e| e.id).collect::<Vec<_>>();

    // request would error with 0 exercise ids
    if !exercise_ids.is_empty() {
        let mut server_exercises = client
            .get_exercises_details(&exercise_ids)?
            .into_iter()
            .map(|e| (e.id, e))
            .collect::<HashMap<_, _>>();
        for local_exercise in local_exercises {
            let server_exercise = server_exercises
                .remove(&local_exercise.id)
                .ok_or(LangsError::ExerciseMissingOnServer(local_exercise.id))?;
            if server_exercise.checksum != local_exercise.checksum {
                // server has an updated exercise
                let target = ProjectsConfig::get_exercise_download_target(
                    &projects_dir,
                    &server_exercise.course_name,
                    &server_exercise.exercise_name,
                );
                exercises_to_update.push(ExerciseDownload {
                    id: server_exercise.id,
                    course_slug: server_exercise.course_name.clone(),
                    exercise_slug: server_exercise.exercise_name.clone(),
                    path: target,
                });
                *local_exercise = ProjectsDirExercise {
                    id: server_exercise.id,
                    checksum: server_exercise.checksum,
                };
            }
        }

        if !exercises_to_update.is_empty() {
            for exercise in &exercises_to_update {
                client.download_exercise(exercise.id, &exercise.path)?;
            }

            for (course_name, exercise_names) in course_data {
                let mut exercises = BTreeMap::new();
                for (exercise_name, checksum, id) in exercise_names {
                    exercises.insert(exercise_name, ProjectsDirExercise { id, checksum });
                }

                if let Some(course_config) = projects_config.courses.get_mut(&course_name) {
                    course_config.exercises.extend(exercises);
                    course_config.save_to_projects_dir(&projects_dir)?;
                } else {
                    let course_config = CourseConfig {
                        course: course_name,
                        exercises,
                    };
                    course_config.save_to_projects_dir(&projects_dir)?;
                };
            }
        }
    }

    Ok(DownloadOrUpdateCourseExercisesResult {
        downloaded: exercises_to_update,
        skipped: vec![],
    })
}

/// Fetches a setting from the config.
pub fn get_setting(client_name: &str, key: &str) -> Result<ConfigValue<'static>, LangsError> {
    log::debug!("fetching setting {} in {}", key, client_name);

    let tmc_config = get_settings(client_name)?;
    let value = tmc_config.get(key).into_owned();
    Ok(value)
}

/// Fetches all the settings from the config.
pub fn get_settings(client_name: &str) -> Result<TmcConfig, LangsError> {
    log::debug!("fetching settings for {}", client_name);

    let config_path = TmcConfig::get_location(client_name)?;
    TmcConfig::load(client_name, &config_path)
}

/// Saves a setting in the config.
pub fn set_setting(client_name: &str, key: &str, value: &str) -> Result<(), LangsError> {
    log::debug!("setting {}={} in {}", key, value, client_name);

    let config_path = TmcConfig::get_location(client_name)?;
    let mut tmc_config = TmcConfig::load(client_name, &config_path)?;
    let value = match serde_json::from_str(value) {
        Ok(json) => json,
        Err(_) => {
            // interpret as string
            JsonValue::String(value.to_string())
        }
    };
    let value = setting_json_to_toml(value)?;

    tmc_config.insert(key.to_string(), value.clone())?;
    tmc_config.save(&config_path)?;
    Ok(())
}

/// Resets all settings in the config, removing those without a default value.
pub fn reset_settings(client_name: &str) -> Result<(), LangsError> {
    log::debug!("resetting settings in {}", client_name);

    TmcConfig::reset(client_name)?;
    Ok(())
}

/// Unsets the given setting.
pub fn unset_setting(client_name: &str, key: &str) -> Result<(), LangsError> {
    log::debug!("unsetting setting {} in {}", key, client_name);

    let config_path = TmcConfig::get_location(client_name)?;
    let mut tmc_config = TmcConfig::load(client_name, &config_path)?;

    tmc_config.remove(key)?;
    tmc_config.save(&config_path)?;
    Ok(())
}

fn setting_json_to_toml(json: JsonValue) -> Result<TomlValue, LangsError> {
    match json {
        JsonValue::Array(arr) => {
            let mut v = vec![];
            for value in arr {
                v.push(setting_json_to_toml(value)?);
            }
            Ok(TomlValue::Array(v))
        }
        JsonValue::Bool(b) => Ok(TomlValue::Boolean(b)),
        JsonValue::Null => Err(LangsError::SettingsCannotContainNull),
        JsonValue::Number(num) => {
            if let Some(int) = num.as_i64() {
                Ok(TomlValue::Integer(int))
            } else if let Some(float) = num.as_f64() {
                Ok(TomlValue::Float(float))
            } else {
                // this error can occur because serde_json supports u64 ints but toml doesn't
                Err(LangsError::SettingNumberTooHigh(num))
            }
        }
        JsonValue::Object(obj) => {
            let mut map = TomlMap::new();
            for (key, value) in obj {
                map.insert(key, setting_json_to_toml(value)?);
            }
            Ok(TomlValue::Table(map))
        }
        JsonValue::String(s) => Ok(TomlValue::String(s)),
    }
}

/// Checks the exercise's code quality.
pub fn checkstyle(
    exercise_path: &Path,
    locale: Language,
) -> Result<Option<StyleValidationResult>, LangsError> {
    log::debug!("checking code style in {}", exercise_path.display());

    let style_validation_result = tmc_langs_plugins::get_language_plugin(exercise_path)?
        .check_code_style(exercise_path, locale)?;
    Ok(style_validation_result)
}

/// Cleans the exercise.
pub fn clean(exercise_path: &Path) -> Result<(), LangsError> {
    log::debug!("cleaning {}", exercise_path.display());

    tmc_langs_plugins::get_language_plugin(exercise_path)?.clean(exercise_path)?;
    Ok(())
}

/// Compresses the exercise to the target path.
pub fn compress_project_to(source: &Path, target: &Path) -> Result<(), LangsError> {
    log::debug!("compressing {} to {}", source.display(), target.display());

    let data = tmc_langs_plugins::compress_project(source)?;

    if let Some(parent) = target.parent() {
        file_util::create_dir_all(parent)?;
    }
    file_util::write_to_file(&data, target)?;
    Ok(())
}

/*
/// Checks how many megabytes are available on the disk containing the target path.
pub fn free_disk_space_megabytes(path: &Path) -> Result<u64, LangsError> {
    log::debug!("checking disk usage in {}", path.display());

    let usage = smol::block_on(disk::usage(path))?
        .free()
        .get::<heim::units::information::megabyte>();
    Ok(usage)
}
*/

/// Extracts the compressed project to the target location.
pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    clean: bool,
) -> Result<(), LangsError> {
    log::debug!(
        "extracting compressed project to {}",
        target_location.display()
    );

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

/// Parses the available points from the exercise.
pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, LangsError> {
    log::debug!("parsing available points in {}", exercise_path.display());

    let points = tmc_langs_plugins::get_language_plugin(exercise_path)?
        .get_available_points(exercise_path)?;
    Ok(points)
}

/// Finds valid exercises from the given path.
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

/// Gets the exercise packaging configuration.
pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, LangsError> {
    log::debug!("getting exercise packaging config for {}", path.display());

    let config = TmcProjectYml::from(path)?;
    Ok(tmc_langs_plugins::get_language_plugin(path)?
        .get_exercise_packaging_configuration(config)?)
}

/// Prepares the exercise stub, copying tmc-junit-runner for Ant exercises.
pub fn prepare_stub(exercise_path: &Path, dest_path: &Path) -> Result<(), LangsError> {
    log::debug!(
        "preparing stub for {} in {}",
        exercise_path.display(),
        dest_path.display()
    );

    submission_processing::prepare_stub(&exercise_path, dest_path)?;

    // The Ant plugin needs some additional files to be copied over.
    if let Plugin::Ant(_) = tmc_langs_plugins::get_language_plugin(exercise_path)? {
        AntPlugin::copy_tmc_junit_runner(dest_path).map_err(|e| TmcError::Plugin(Box::new(e)))?;
    }
    Ok(())
}

/// Runs tests for the exercise.
pub fn run_tests(path: &Path) -> Result<RunResult, LangsError> {
    log::debug!("running tests in {}", path.display());

    Ok(tmc_langs_plugins::get_language_plugin(path)?.run_tests(path)?)
}

/// Scans the exercise.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, LangsError> {
    log::debug!("scanning exercise in {}", path.display());

    Ok(tmc_langs_plugins::get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

/// Extracts student files from the compressed exercise.
pub fn extract_student_files(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), LangsError> {
    log::debug!(
        "extracting student files from compressed project to {}",
        target_location.display()
    );

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