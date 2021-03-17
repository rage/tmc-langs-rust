#![deny(clippy::print_stdout, clippy::print_stderr)]

//! The main tmc-langs library.

pub mod config;
pub mod course_refresher;
pub mod data;
mod error;
mod submission_packaging;
mod submission_processing;

pub use crate::course_refresher::refresh_course;
pub use crate::data::{DownloadResult, ExerciseDownload};
pub use crate::submission_packaging::prepare_submission;
pub use crate::submission_processing::prepare_solution;
use data::{CombinedCourseData, DownloadOrUpdateCourseExercisesResult};
use oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
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
use config::{ConfigValue, CourseConfig, Credentials, Exercise};
use heim::disk;
use serde_json::Value as JsonValue;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use std::{collections::HashMap, ffi::OsStr};
use tmc_langs_framework::{NothingIsStudentFilePolicy, StudentFilePolicy, TmcError, TmcProjectYml};
use tmc_langs_plugins::tmc_zip;
use tmc_langs_util::progress_reporter;
use toml::{map::Map as TomlMap, Value as TomlValue};
use url::Url;
use walkdir::WalkDir;

pub fn check_exercise_updates(
    client: &TmcClient,
    client_name: &str,
) -> Result<Vec<usize>, LangsError> {
    let mut updated_exercises = vec![];

    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    let config = ProjectsConfig::load(&projects_dir)?;
    let local_exercises = config
        .courses
        .into_iter()
        .map(|c| c.1.exercises)
        .flatten()
        .map(|e| e.1)
        .collect::<Vec<_>>();

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

pub fn download_old_submission(
    client: &TmcClient,
    exercise_id: usize,
    output_path: PathBuf,
    submission_id: usize,
    submission_url: Option<Url>,
) -> Result<(), LangsError> {
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

pub fn download_or_update_course_exercises(
    client: &TmcClient,
    client_name: &str,
    exercises: &[usize],
    download_from_template: bool,
) -> Result<DownloadResult, LangsError> {
    let exercises_details = client.get_exercises_details(exercises)?;

    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    let mut projects_config = ProjectsConfig::load(&projects_dir)?;

    // separate exercises into fresh downloads, submission downloads and skipped
    let mut to_be_downloaded_fresh = HashMap::new();
    let mut to_be_downloaded_submission = HashMap::new();
    let mut to_be_skipped = vec![];

    for exercise_detail in exercises_details {
        let target = ProjectsConfig::get_exercise_download_target(
            &projects_dir,
            &exercise_detail.course_name,
            &exercise_detail.exercise_name,
        );

        // check if the exercise is already on disk
        if let Some(course_config) = projects_config.courses.get(&exercise_detail.course_name) {
            if let Some(exercise) = course_config.exercises.get(&exercise_detail.exercise_name) {
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
                        course_slug: exercise_detail.course_name,
                        exercise_slug: exercise_detail.exercise_name,
                        path: target,
                    });
                    continue;
                } else if !download_from_template {
                    // different checksum, if flag isn't set check if there are any previous submissions
                    if let Some(latest_submission) = client
                        .get_exercise_submissions_for_current_user(exercise.id)?
                        .first()
                    {
                        // previous submission found
                        to_be_downloaded_submission.insert(
                            exercise_detail.id,
                            ExerciseDownload {
                                course_slug: exercise_detail.exercise_name.clone(),
                                exercise_slug: exercise_detail.exercise_name.clone(),
                                path: target,
                            },
                        );
                        continue;
                    }
                }
            }
        }
        // not skipped, should be downloaded
        // also store id and checksum to be used later
        to_be_downloaded.insert(
            exercise_detail.id,
            (
                ExerciseDownload {
                    course_slug: exercise_detail.course_name.clone(),
                    exercise_slug: exercise_detail.exercise_name.clone(),
                    path: target,
                },
                exercise_detail.id,
                exercise_detail.checksum,
            ),
        );
    }

    // download and divide the results into successful and failed downloads
    let exercises_and_paths = to_be_downloaded
        .iter()
        .map(|(id, (ex, ..))| (*id, ex.path.clone()))
        .collect();
    let download_result = client.download_or_update_exercises(exercises_and_paths);
    let (downloaded, failed) = match download_result {
        Ok(_) => {
            let downloaded = to_be_downloaded.into_iter().map(|(_, v)| v).collect();
            let failed = vec![];
            (downloaded, failed)
        }
        Err(ClientError::IncompleteDownloadResult { downloaded, failed }) => {
            let downloaded = downloaded
                .iter()
                .map(|id| to_be_downloaded.remove(id).unwrap())
                .collect::<Vec<_>>();
            let failed = failed
                .into_iter()
                .map(|(id, e)| (to_be_downloaded.remove(&id).unwrap(), e))
                .collect::<Vec<_>>();
            (downloaded, failed)
        }
        Err(error) => {
            return Err(LangsError::TmcClient(error));
        }
    };

    // turn the downloaded exercises into a hashmap with the course as key
    let mut course_data = HashMap::<String, Vec<(String, String, usize)>>::new();
    for (download, id, checksum) in &downloaded {
        let entry = course_data.entry(download.course_slug.clone());
        let course_exercises = entry.or_default();
        course_exercises.push((download.exercise_slug.clone(), checksum.clone(), *id));
    }
    // update/create the course configs that contain downloaded or updated exercises
    for (course_name, exercise_names) in course_data {
        let exercises = exercise_names
            .into_iter()
            .map(|(name, checksum, id)| (name, Exercise { id, checksum }))
            .collect();
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

    let completed = downloaded.into_iter().map(|d| d.0).collect();
    // return an error if any downloads failed
    if !failed.is_empty() {
        // add an error trace to each failed download
        let failed = failed
            .into_iter()
            .map(|((ex, ..), err)| {
                let mut error = &err as &dyn std::error::Error;
                let mut chain = vec![error.to_string()];
                while let Some(source) = error.source() {
                    chain.push(source.to_string());
                    error = source;
                }
                (ex, chain)
            })
            .collect();
        return Ok(DownloadResult::Failure {
            downloaded: completed,
            skipped: to_be_skipped,
            failed,
        });
    }

    Ok(DownloadResult::Success {
        downloaded: completed,
        skipped: to_be_skipped,
    })
}

pub fn get_course_data(
    client: &TmcClient,
    course_id: usize,
) -> Result<CombinedCourseData, LangsError> {
    let details = client.get_course_details(course_id)?;
    let exercises = client.get_course_exercises(course_id)?;
    let settings = client.get_course(course_id)?;
    Ok(CombinedCourseData {
        details,
        exercises,
        settings,
    })
}

pub fn login_with_token(token: String) -> Token {
    let mut token_response = StandardTokenResponse::new(
        AccessToken::new(token),
        BasicTokenType::Bearer,
        EmptyExtraTokenFields {},
    );
    token_response.set_scopes(Some(vec![Scope::new("public".to_string())]));
    token_response
}

pub fn login_with_password(
    client: &mut TmcClient,
    base64: bool,
    client_name: &str,
    email: String,
) -> Result<Token, LangsError> {
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

pub fn update_exercises(
    client: &TmcClient,
    client_name: &str,
) -> Result<DownloadOrUpdateCourseExercisesResult, LangsError> {
    let exercises_to_update = vec![];
    let mut to_be_downloaded = vec![];
    let mut to_be_skipped = vec![];
    let mut course_data = HashMap::<String, Vec<(String, String, usize)>>::new();

    let config_path = TmcConfig::get_location(client_name)?;
    let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
    let mut projects_config = ProjectsConfig::load(&projects_dir)?;
    let local_exercises = projects_config
        .courses
        .iter()
        .map(|c| &c.1.exercises)
        .flatten()
        .map(|e| e.1)
        .collect::<Vec<_>>();
    let exercise_ids = local_exercises.iter().map(|e| e.id).collect::<Vec<_>>();

    // request would error with 0 exercise ids
    if !exercise_ids.is_empty() {
        let server_exercises = client
            .get_exercises_details(&exercise_ids)?
            .into_iter()
            .map(|e| (e.id, e))
            .collect::<HashMap<_, _>>();
        for local_exercise in local_exercises {
            let server_exercise = server_exercises
                .get(&local_exercise.id)
                .ok_or(LangsError::ExerciseMissingOnServer(local_exercise.id))?;
            let target = ProjectsConfig::get_exercise_download_target(
                &projects_dir,
                &server_exercise.course_name,
                &server_exercise.exercise_name,
            );
            if server_exercise.checksum != local_exercise.checksum {
                // server has an updated exercise
                let exercise_list = course_data
                    .entry(server_exercise.course_name.clone())
                    .or_default();
                exercise_list.push((
                    server_exercise.exercise_name.clone(),
                    server_exercise.checksum.clone(),
                    server_exercise.id,
                ));
                to_be_downloaded.push(ExerciseDownload {
                    course_slug: server_exercise.course_name.clone(),
                    exercise_slug: server_exercise.exercise_name.clone(),
                    path: target,
                });
            } else {
                to_be_skipped.push(ExerciseDownload {
                    course_slug: server_exercise.course_name.clone(),
                    exercise_slug: server_exercise.exercise_name.clone(),
                    path: target,
                });
            }
        }

        if !exercises_to_update.is_empty() {
            client.download_or_update_exercises(exercises_to_update)?;

            for (course_name, exercise_names) in course_data {
                let mut exercises = BTreeMap::new();
                for (exercise_name, checksum, id) in exercise_names {
                    exercises.insert(exercise_name, Exercise { id, checksum });
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
        downloaded: to_be_downloaded,
        skipped: to_be_skipped,
    })
}

pub fn get_setting(client_name: &str, key: &str) -> Result<ConfigValue<'static>, LangsError> {
    let tmc_config = get_settings(client_name)?;
    let value = tmc_config.get(key).into_owned();
    Ok(value)
}

pub fn get_settings(client_name: &str) -> Result<TmcConfig, LangsError> {
    let config_path = TmcConfig::get_location(client_name)?;
    TmcConfig::load(client_name, &config_path)
}

pub fn set_setting(client_name: &str, key: &str, value: &str) -> Result<(), LangsError> {
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

pub fn reset_settings(client_name: &str) -> Result<(), LangsError> {
    TmcConfig::reset(client_name)?;
    Ok(())
}

pub fn unset_setting(client_name: &str, key: &str) -> Result<(), LangsError> {
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
