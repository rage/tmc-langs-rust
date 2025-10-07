#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! The main tmc-langs library. Provides a convenient API to all of the functionality provided by the tmc-langs project.

mod config;
mod course_refresher;
mod data;
mod error;
mod submission_packaging;
mod submission_processing;

use crate::data::{DownloadTarget, DownloadTargetKind};
pub use crate::{
    config::{
        Credentials, ProjectsConfig, ProjectsDirTmcExercise, TmcConfig, TmcCourseConfig,
        list_local_tmc_course_exercises, migrate_exercise, move_projects_dir,
    },
    course_refresher::{RefreshData, RefreshExercise, refresh_course},
    data::{
        CombinedCourseData, ConfigValue, DownloadOrUpdateMoocCourseExercisesResult,
        DownloadOrUpdateTmcCourseExercisesResult, DownloadResult, LocalExercise, LocalMoocExercise,
        LocalTmcExercise, MoocExerciseDownload, TmcExerciseDownload, TmcParams,
    },
    error::{LangsError, ParamError},
    submission_packaging::{PrepareSubmission, prepare_submission},
    submission_processing::prepare_solution,
};
use hmac::{Hmac, Mac};
// use heim::disk;
use jwt::SignWithKey;
use oauth2::{
    AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse, basic::BasicTokenType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, HashMap},
    convert::TryFrom,
    ffi::OsStr,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tmc_langs_framework::Archive;
pub use tmc_langs_framework::{
    CommandError, Compression, ExerciseDesc, ExercisePackagingConfiguration, Language,
    LanguagePlugin, PythonVer, RunResult, RunStatus, StyleValidationError, StyleValidationResult,
    StyleValidationStrategy, TestDesc, TestResult, TmcProjectYml,
};
use tmc_langs_plugins::{
    CSharpPlugin, MakePlugin, NoTestsPlugin, Plugin, PluginType, Python3Plugin, RPlugin,
};
use tmc_langs_util::file_util::LOCK_FILE_NAME;
// the Java plugin is disabled on musl
pub use tmc_langs_util::{file_util, notification_reporter, progress_reporter};
pub use tmc_mooc_client as mooc;
use tmc_mooc_client::{MoocClient, api::ExerciseUpdateData};
pub use tmc_testmycode_client as tmc;
use toml::Value as TomlValue;
use url::Url;
use walkdir::WalkDir;
#[cfg(not(target_env = "musl"))]
use {
    tmc_langs_framework::TmcError,
    tmc_langs_plugins::{AntPlugin, MavenPlugin},
};

const TMC_LANGS_CONFIG_DIR_VAR: &str = "TMC_LANGS_CONFIG_DIR";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct UpdatedExercise {
    pub id: u32,
}

/// Signs the given serializable value with the given secret using JWT.
///
/// # Example
/// ```
/// #[derive(serde::Serialize)]
/// struct TestResult {
///     passed: bool,
/// }
///
/// let token = tmc_langs::sign_with_jwt(TestResult { passed: true }, "secret".as_bytes()).unwrap();
/// assert_eq!(token, "eyJhbGciOiJIUzI1NiJ9.eyJwYXNzZWQiOnRydWV9.y-jXHgxZ_5wRqursLTb1hJOYob6LKj0mYBPnZSGtsnU");
/// ```
///
/// # Errors
/// Should never fail, but returns an error to be safe against changes in external libraries.
pub fn sign_with_jwt<T: Serialize>(value: T, secret: &[u8]) -> Result<String, LangsError> {
    let key: Hmac<Sha256> = Hmac::<Sha256>::new_from_slice(secret)?;
    let token = value.sign_with_key(&key)?;
    Ok(token)
}

/// Returns the projects directory for the given client name.
/// The return value for `my-client` might look something like `/home/username/.local/share/tmc/my-client` on Linux.
pub fn get_projects_dir(client_name: &str) -> Result<PathBuf, LangsError> {
    let projects_dir = TmcConfig::load(client_name)?.projects_dir;
    Ok(projects_dir)
}

/// Checks the server for any updates for exercises within the given projects directory.
/// Returns the ids of each exercise that can be updated.
pub fn check_exercise_updates(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
) -> Result<Vec<u32>, LangsError> {
    log::debug!("checking exercise updates in {}", projects_dir.display());

    let mut updated_exercises = vec![];

    let config = ProjectsConfig::load(projects_dir)?;
    let local_exercises = config.get_all_tmc_exercises().collect::<Vec<_>>();

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
                .ok_or(LangsError::ExerciseMissingOnServer(local_exercise.id))?;
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
    client: &tmc::TestMyCodeClient,
    exercise_id: u32,
    output_path: &Path,
    submission_id: u32,
    save_old_state: bool,
) -> Result<(), LangsError> {
    log::debug!("downloading old submission {submission_id} for {exercise_id}");

    if save_old_state {
        // submit old exercise
        let tmc_project_yml = TmcProjectYml::load_or_default(output_path)?;
        client.submit(
            exercise_id,
            output_path,
            tmc_project_yml.get_submission_size_limit_mb(),
            None,
        )?;
        log::debug!("finished submission");
    }

    // reset old exercise
    reset(client, exercise_id, output_path)?;
    log::debug!("reset exercise");

    // dl submission
    let mut buf = vec![];
    client.download_old_submission(submission_id, &mut buf)?;
    log::debug!("downloaded old submission");

    // extract submission
    extract_student_files(Cursor::new(buf), Compression::Zip, output_path)?;
    log::debug!("extracted project");
    Ok(())
}

/// Submits the exercise to the server
pub fn submit_exercise(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
    course_slug: &str,
    exercise_slug: &str,
    locale: Option<Language>,
) -> Result<tmc::response::NewSubmission, LangsError> {
    let projects_config = ProjectsConfig::load(projects_dir)?;
    let exercise = projects_config
        .get_tmc_exercise(course_slug, exercise_slug)
        .ok_or(LangsError::NoProjectExercise)?;

    let exercise_path =
        ProjectsConfig::get_tmc_exercise_download_target(projects_dir, course_slug, exercise_slug);

    let tmc_project_yml = TmcProjectYml::load_or_default(&exercise_path)?;
    client
        .submit(
            exercise.id,
            exercise_path.as_path(),
            tmc_project_yml.get_submission_size_limit_mb(),
            locale,
        )
        .map_err(Into::into)
}

/// Sends the paste to the server
pub fn paste_exercise(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
    course_slug: &str,
    exercise_slug: &str,
    paste_message: Option<String>,
    locale: Option<Language>,
) -> Result<tmc::response::NewSubmission, LangsError> {
    let projects_config = ProjectsConfig::load(projects_dir)?;
    let exercise = projects_config
        .get_tmc_exercise(course_slug, exercise_slug)
        .ok_or(LangsError::NoProjectExercise)?;

    let exercise_path =
        ProjectsConfig::get_tmc_exercise_download_target(projects_dir, course_slug, exercise_slug);

    let tmc_project_yml = TmcProjectYml::load_or_default(&exercise_path)?;
    client
        .paste(
            exercise.id,
            exercise_path.as_path(),
            paste_message,
            locale,
            tmc_project_yml.get_submission_size_limit_mb(),
        )
        .map_err(Into::into)
}

/// Downloads the given exercises, by either downloading the exercise template, updating the exercise or downloading an old submission.
/// Requires authentication.
/// If the exercise doesn't exist on disk yet...
///   if there are previous submissions and download_template is not set, the latest submission is downloaded.
///   otherwise, the exercise template is downloaded.
/// If the exercise exists on disk, it is updated using the course template.
pub fn download_or_update_course_exercises(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
    exercises: &[u32],
    download_template: bool,
) -> Result<DownloadResult, LangsError> {
    log::debug!(
        "downloading or updating course exercises in {}",
        projects_dir.display()
    );

    client.require_authentication().map_err(Box::new)?;

    let exercises_details = client.get_exercises_details(exercises)?;
    let projects_config = ProjectsConfig::load(projects_dir)?;

    // separate exercises into downloads and skipped
    let mut to_be_downloaded = vec![];
    let mut to_be_skipped = vec![];

    log::debug!("checking the checksum of each exercise on the server");
    for exercise_detail in exercises_details {
        let target = ProjectsConfig::get_tmc_exercise_download_target(
            projects_dir,
            &exercise_detail.course_name,
            &exercise_detail.exercise_name,
        );

        // check if the exercise is already on disk
        if let Some(exercise) = projects_config
            .get_tmc_exercise(&exercise_detail.course_name, &exercise_detail.exercise_name)
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
                to_be_skipped.push(TmcExerciseDownload {
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
                    // previous submission found, check if exercise submission results hidden (part of exam)
                    if !exercise_detail.hide_submission_results {
                        to_be_downloaded.push(DownloadTarget {
                            target: TmcExerciseDownload {
                                id: exercise_detail.id,
                                course_slug: exercise_detail.course_name,
                                exercise_slug: exercise_detail.exercise_name,
                                path: target,
                            },
                            checksum: exercise_detail.checksum,
                            kind: DownloadTargetKind::Submission {
                                submission_id: latest_submission.id,
                            },
                        });
                        continue;
                    }
                }
            }
        }

        // not skipped, either not on disk or no previous submissions or submission result hidden, downloading template
        to_be_downloaded.push(DownloadTarget {
            target: TmcExerciseDownload {
                id: exercise_detail.id,
                course_slug: exercise_detail.course_name.clone(),
                exercise_slug: exercise_detail.exercise_name.clone(),
                path: target,
            },
            checksum: exercise_detail.checksum,
            kind: DownloadTargetKind::Template,
        });
    }

    let exercises_len = to_be_downloaded.len();
    progress_reporter::start_stage::<()>(
        u32::try_from(exercises_len).expect("should never happen") * 2 + 1, // each download progresses at 2 points, plus the final finishing step
        format!("Downloading {exercises_len} exercises"),
        None,
    );

    log::debug!("downloading exercises");
    // download and divide the results into successful and failed downloads
    let thread_count = to_be_downloaded.len().min(4); // max 4 threads
    let mut handles = vec![];
    let exercises = Arc::new(Mutex::new(to_be_downloaded));
    let projects_config = Arc::new(Mutex::new(projects_config));
    for _thread_id in 0..thread_count {
        let client = client.clone();
        let exercises = Arc::clone(&exercises);
        let projects_config = Arc::clone(&projects_config);
        let projects_dir = projects_dir.to_path_buf();

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
                    progress_reporter::progress_stage::<tmc::ClientUpdateData>(
                        format!(
                            "Downloading exercise {} to '{}'",
                            download_target.target.id,
                            download_target.target.path.display(),
                        ),
                        Some(tmc::ClientUpdateData::ExerciseDownload {
                            id: download_target.target.id,
                            path: download_target.target.path.clone(),
                        }),
                    );

                    // execute download based on type
                    match &download_target.kind {
                        DownloadTargetKind::Template => {
                            let mut buf = vec![];
                            client.download_exercise(download_target.target.id, &mut buf)?;
                            extract_project(
                                Cursor::new(buf),
                                &download_target.target.path,
                                Compression::Zip,
                                false,
                                false,
                            )?;
                        }
                        DownloadTargetKind::Submission { submission_id } => {
                            let mut buf = vec![];
                            client.download_exercise(download_target.target.id, &mut buf)?;
                            extract_project(
                                Cursor::new(buf),
                                &download_target.target.path,
                                Compression::Zip,
                                false,
                                false,
                            )?;

                            let plugin = PluginType::from_exercise(&download_target.target.path)?;
                            let config = plugin.get_exercise_packaging_configuration(
                                &download_target.target.path,
                            )?;
                            for student_file in config.student_file_paths {
                                let student_file = download_target.target.path.join(student_file);
                                if student_file.is_file() {
                                    file_util::remove_file(&student_file)?;
                                } else {
                                    file_util::remove_dir_all(&student_file)?;
                                }
                            }

                            let mut buf = vec![];
                            client.download_old_submission(*submission_id, &mut buf)?;
                            if let Err(err) = plugin.extract_student_files(
                                Cursor::new(buf),
                                Compression::Zip,
                                &download_target.target.path,
                            ) {
                                log::error!(
                                    "Something went wrong when downloading old submission: {err}"
                                );
                            }
                        }
                    }
                    // download successful, save to course config
                    let mut projects_config =
                        projects_config.lock().map_err(|_| LangsError::MutexError)?; // lock mutex
                    let course_config = projects_config
                        .get_or_init_tmc_course_config(download_target.target.course_slug.clone());
                    course_config.add_exercise(
                        download_target.target.exercise_slug.clone(),
                        download_target.target.id,
                        download_target.checksum.clone(),
                    );
                    course_config.save_to_projects_dir(&projects_dir)?;
                    drop(projects_config); // drop mutex

                    progress_reporter::progress_stage::<tmc::ClientUpdateData>(
                        format!(
                            "Downloaded exercise {} to '{}'",
                            download_target.target.id,
                            download_target.target.path.display(),
                        ),
                        Some(tmc::ClientUpdateData::ExerciseDownload {
                            id: download_target.target.id,
                            path: download_target.target.path.clone(),
                        }),
                    );

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

    // gather results from each thread
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

    // report
    let finish_message = if failed.is_empty() {
        if successful.is_empty() && exercises_len == 0 {
            "Exercises are already up-to-date!".to_string()
        } else {
            format!(
                "Successfully downloaded {} out of {} exercises.",
                successful.len(),
                exercises_len
            )
        }
    } else {
        format!(
            "Downloaded {} out of {} exercises ({} failed)",
            successful.len(),
            exercises_len,
            failed.len(),
        )
    };
    progress_reporter::finish_stage::<tmc::ClientUpdateData>(finish_message, None);

    // return information about the downloads
    let downloaded = successful.into_iter().map(|t| t.target).collect();
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
                (target.target, chain)
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
    client: &tmc::TestMyCodeClient,
    course_id: u32,
) -> Result<CombinedCourseData, LangsError> {
    log::debug!("getting course data for {course_id}");

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
pub fn login_with_token(token: String) -> tmc::Token {
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
    client: &mut tmc::TestMyCodeClient,
    email: String,
    password: String,
) -> Result<tmc::Token, LangsError> {
    log::debug!("logging in with password");
    let token = client.authenticate(email, password)?;
    Ok(token)
}

/// Initializes a TestMyCodeClient, using and returning the stored credentials, if any.
pub fn init_testmycode_client_with_credentials(
    root_url: Url,
    client_name: &str,
    client_version: &str,
) -> Result<(tmc::TestMyCodeClient, Option<Credentials>), LangsError> {
    // create client
    let mut client = tmc::TestMyCodeClient::new(
        root_url,
        client_name.to_string(),
        client_version.to_string(),
    )?;

    // set token from the credentials file if one exists
    let credentials = Credentials::load(client_name)?;
    if let Some(credentials) = &credentials {
        client.set_token(credentials.token());
    }

    Ok((client, credentials))
}

/// Initializes a MoocClient, using and returning the stored credentials, if any.
pub fn init_mooc_client_with_credentials(
    root_url: Url,
    client_name: &str,
) -> Result<(mooc::MoocClient, Option<Credentials>), LangsError> {
    // create client
    let mut client = mooc::MoocClient::new(root_url);

    // set token from the credentials file if one exists
    let credentials = Credentials::load(client_name)?;
    if let Some(credentials) = &credentials {
        client.set_token(credentials.token());
    }

    Ok((client, credentials))
}

/// Updates the tmc exercises in the local projects directory.
// TODO: parallel downloads
pub fn update_tmc_exercises(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
) -> Result<DownloadOrUpdateTmcCourseExercisesResult, LangsError> {
    log::debug!("updating exercises in {}", projects_dir.display());

    let mut course_data = HashMap::<String, Vec<(String, String, u32)>>::new();

    let mut projects_config = ProjectsConfig::load(projects_dir)?;

    let exercises = projects_config
        .tmc_courses
        .values()
        .flat_map(|cc| cc.exercises.values())
        .collect::<Vec<_>>();

    let mut exercises_to_update = vec![];
    // request would error with 0 exercise ids
    if !exercises.is_empty() {
        let tmc_exercise_ids = exercises.iter().map(|e| e.id).collect::<Vec<_>>();
        let mut tmc_server_exercises = client
            .get_exercises_details(&tmc_exercise_ids)?
            .into_iter()
            .map(|e| (e.id, e))
            .collect::<HashMap<_, _>>();

        // first, handle tmc
        for course_config in projects_config.tmc_courses.values_mut() {
            for local_exercise in course_config.exercises.values_mut() {
                let server_exercise = tmc_server_exercises
                    .remove(&local_exercise.id)
                    .ok_or(LangsError::ExerciseMissingOnServer(local_exercise.id))?;
                if server_exercise.checksum != local_exercise.checksum {
                    // server has an updated exercise
                    let target = ProjectsConfig::get_tmc_exercise_download_target(
                        projects_dir,
                        &server_exercise.course_name,
                        &server_exercise.exercise_name,
                    );
                    exercises_to_update.push(TmcExerciseDownload {
                        id: server_exercise.id,
                        course_slug: server_exercise.course_name.clone(),
                        exercise_slug: server_exercise.exercise_name.clone(),
                        path: target,
                    });
                    *local_exercise = ProjectsDirTmcExercise {
                        id: server_exercise.id,
                        checksum: server_exercise.checksum,
                    };
                }
                let data = course_data.entry(course_config.course.clone()).or_default();
                data.push((
                    server_exercise.exercise_name,
                    local_exercise.checksum.clone(),
                    local_exercise.id,
                ));
            }
        }
        if !exercises_to_update.is_empty() {
            for exercise in &exercises_to_update {
                let mut buf = vec![];
                client.download_exercise(exercise.id, &mut buf)?;
                extract_project(
                    Cursor::new(buf),
                    &exercise.path,
                    Compression::Zip,
                    false,
                    false,
                )?;
            }
            for (course_name, exercise_names) in course_data {
                let mut exercises = BTreeMap::new();
                for (exercise_name, checksum, id) in exercise_names {
                    exercises.insert(exercise_name, ProjectsDirTmcExercise { id, checksum });
                }

                if let Some(course_config) = projects_config.tmc_courses.get_mut(&course_name) {
                    course_config.exercises.extend(exercises);
                    course_config.save_to_projects_dir(projects_dir)?;
                } else {
                    let course_config = TmcCourseConfig {
                        course: course_name,
                        exercises,
                    };
                    course_config.save_to_projects_dir(projects_dir)?;
                };
            }
        }
    }

    Ok(DownloadOrUpdateTmcCourseExercisesResult {
        downloaded: exercises_to_update,
        skipped: vec![],
        failed: None,
    })
}

/// Updates the mooc exercises in the local projects directory.
pub fn update_mooc_exercises(
    client: &MoocClient,
    projects_dir: &Path,
) -> Result<DownloadOrUpdateMoocCourseExercisesResult, LangsError> {
    let projects_config = ProjectsConfig::load(projects_dir)?;
    let exercises = projects_config
        .mooc_courses
        .values()
        .map(|cc| (cc, &cc.exercises))
        .flat_map(|(cc, cce)| cce.values().map(move |e| (e.id, (cc, e))))
        .collect::<HashMap<_, _>>();

    let mut downloaded = Vec::new();
    if !exercises.is_empty() {
        let exercise_update_data = exercises
            .values()
            .map(|(_c, e)| ExerciseUpdateData {
                id: e.id,
                checksum: &e.checksum,
            })
            .collect::<Vec<_>>();
        let exercise_updates = client.check_exercise_updates(&exercise_update_data)?;
        for updated_exercise in exercise_updates.updated_exercises {
            if let Some((course, exercise)) = exercises.get(&updated_exercise) {
                let target = ProjectsConfig::get_mooc_exercise_download_target(
                    projects_dir,
                    &course.directory,
                    &exercise.directory,
                );
                let data = client.download_exercise(updated_exercise)?;
                extract_project(Cursor::new(data), &target, Compression::Zip, false, false)?;
                downloaded.push(MoocExerciseDownload {
                    id: updated_exercise,
                    path: target,
                })
            } else {
                log::warn!("Server returned unexpected exercise id {updated_exercise}");
            }
        }
        for _deleted_exercise in exercise_updates.deleted_exercises {
            // todo
        }
    }

    Ok(DownloadOrUpdateMoocCourseExercisesResult {
        downloaded,
        skipped: vec![],
        failed: None,
    })
}

/// Fetches a setting from the config.
pub fn get_setting(client_name: &str, key: &str) -> Result<ConfigValue, LangsError> {
    log::debug!("fetching setting {key} in {client_name}");

    let tmc_config = get_settings(client_name)?;
    let value = match key {
        "projects-dir" => ConfigValue::Path(tmc_config.get_projects_dir().to_path_buf()),
        other => ConfigValue::Value(tmc_config.get(other).cloned()),
    };
    Ok(value)
}

/// Fetches all the settings from the config.
pub fn get_settings(client_name: &str) -> Result<TmcConfig, LangsError> {
    log::debug!("fetching settings for {client_name}");

    TmcConfig::load(client_name)
}

/// Saves a setting in the config.
pub fn set_setting<T: Serialize>(client_name: &str, key: &str, value: T) -> Result<(), LangsError> {
    log::debug!("setting {key} in {client_name}");

    let mut tmc_config = TmcConfig::load(client_name)?;

    let value = TomlValue::try_from(value)?;
    match key {
        "projects-dir" => {
            let TomlValue::String(value) = value else {
                return Err(LangsError::ProjectsDirNotString);
            };
            tmc_config.set_projects_dir(PathBuf::from(value))?;
        }
        other => {
            tmc_config.insert(other.to_string(), value);
        }
    }

    tmc_config.save()?;
    Ok(())
}

/// Resets all settings in the config, removing those without a default value.
pub fn reset_settings(client_name: &str) -> Result<(), LangsError> {
    log::debug!("resetting settings in {client_name}");

    TmcConfig::reset(client_name)?;
    Ok(())
}

/// Unsets the given setting.
pub fn unset_setting(client_name: &str, key: &str) -> Result<Option<TomlValue>, LangsError> {
    log::debug!("unsetting setting {key} in {client_name}");

    let mut tmc_config = TmcConfig::load(client_name)?;
    let old_value = tmc_config.remove(key);
    tmc_config.save()?;

    Ok(old_value)
}

/// Checks the exercise's code quality.
pub fn checkstyle(
    exercise_path: &Path,
    locale: Language,
) -> Result<Option<StyleValidationResult>, LangsError> {
    log::debug!("checking code style in {}", exercise_path.display());

    let style_validation_result =
        Plugin::from_exercise(exercise_path)?.check_code_style(exercise_path, locale)?;
    Ok(style_validation_result)
}

/// Cleans the exercise.
pub fn clean(exercise_path: &Path) -> Result<(), LangsError> {
    log::debug!("cleaning {}", exercise_path.display());

    Plugin::from_exercise(exercise_path)?.clean(exercise_path)?;
    Ok(())
}

/// Compresses the exercise to the target path.
pub fn compress_project_to(
    source: &Path,
    target: &Path,
    compression: Compression,
    deterministic: bool,
    naive: bool,
) -> Result<(), LangsError> {
    log::debug!(
        "compressing {} to {} ({})",
        source.display(),
        target.display(),
        compression
    );

    let tmc_project_yml = TmcProjectYml::load_or_default(source)?;
    let (data, _hash) = tmc_langs_plugins::compress_project(
        source,
        compression,
        deterministic,
        naive,
        false,
        tmc_project_yml.get_submission_size_limit_mb(),
    )?;
    file_util::write_to_file(data, target)?;
    Ok(())
}

/// Compresses the exercise to the target path.
/// Returns the BLAKE3 hash of the resulting file.
pub fn compress_project_to_with_hash(
    source: &Path,
    target: &Path,
    compression: Compression,
    deterministic: bool,
    naive: bool,
) -> Result<String, LangsError> {
    log::debug!(
        "compressing {} to {} ({})",
        source.display(),
        target.display(),
        compression
    );

    let tmc_project_yml = TmcProjectYml::load_or_default(source)?;
    let (data, hash) = tmc_langs_plugins::compress_project(
        source,
        compression,
        deterministic,
        naive,
        true,
        tmc_project_yml.get_submission_size_limit_mb(),
    )?;
    let hash = hash.expect("set hash to true");
    file_util::write_to_file(data, target)?;
    Ok(hash.to_string())
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

/// Resets the given exercise
pub fn reset(
    client: &tmc::TestMyCodeClient,
    exercise_id: u32,
    exercise_path: &Path,
) -> Result<(), LangsError> {
    if exercise_path.exists() {
        // clear out the exercise directory
        file_util::remove_dir_all(exercise_path)?;
    }
    let mut buf = vec![];
    client.download_exercise(exercise_id, &mut buf)?;
    extract_project(
        Cursor::new(buf),
        exercise_path,
        Compression::Zip,
        false,
        false,
    )?;
    Ok(())
}

/// Extracts the compressed project to the target location.
pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    compression: Compression,
    clean: bool,
    naive: bool,
) -> Result<(), LangsError> {
    log::debug!(
        "extracting compressed project to {}",
        target_location.display()
    );

    if naive {
        extract_project_overwrite(compressed_project, target_location, compression)?;
    } else if let Ok(plugin) = PluginType::from_exercise(target_location) {
        let mut archive = Archive::new(compressed_project, compression)?;
        plugin.extract_project(&mut archive, target_location, clean)?;
    } else {
        let mut archive = Archive::new(compressed_project, compression)?;
        if let Ok(plugin) = PluginType::from_archive(&mut archive) {
            plugin.extract_project(&mut archive, target_location, clean)?;
        } else {
            log::debug!(
                "no matching language plugin found for compressed project, extracting naively",
            );
            let compressed_project = archive.into_inner();
            extract_project_overwrite(compressed_project, target_location, compression)?;
        }
    }
    Ok(())
}

/// Parses the available points from the exercise.
pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, LangsError> {
    log::debug!("parsing available points in {}", exercise_path.display());

    let points = PluginType::from_exercise(exercise_path)?.get_available_points(exercise_path)?;
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
        if Plugin::from_exercise(entry.path()).is_ok() {
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

    let plugin = PluginType::from_exercise(path)?;
    let config = plugin.get_exercise_packaging_configuration(path)?;
    Ok(config)
}

/// Prepares the exercise stub, copying tmc-junit-runner for Ant exercises.
pub fn prepare_stub(exercise_path: &Path, dest_path: &Path) -> Result<(), LangsError> {
    log::debug!(
        "preparing stub for {} in {}",
        exercise_path.display(),
        dest_path.display()
    );

    submission_processing::prepare_stub(exercise_path, dest_path)?;

    // The Ant plugin needs some additional files to be copied over.
    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    if let Ok(PluginType::Ant) = PluginType::from_exercise(exercise_path) {
        AntPlugin::copy_tmc_junit_runner(dest_path).map_err(|e| TmcError::Plugin(Box::new(e)))?;
    }
    Ok(())
}

/// Runs tests for the exercise.
pub fn run_tests(path: &Path) -> Result<RunResult, LangsError> {
    log::debug!("running tests in {}", path.display());

    Ok(Plugin::from_exercise(path)?.run_tests(path)?)
}

/// Scans the exercise.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, LangsError> {
    log::debug!("scanning exercise in {}", path.display());

    Ok(Plugin::from_exercise(path)?.scan_exercise(path, exercise_name)?)
}

/// Extracts student files from the compressed exercise.
pub fn extract_student_files(
    compressed_project: impl std::io::Read + std::io::Seek,
    compression: Compression,
    target_location: &Path,
) -> Result<(), LangsError> {
    log::debug!(
        "extracting student files from compressed project to {}",
        target_location.display()
    );

    if let Ok(plugin) = PluginType::from_exercise(target_location) {
        plugin.extract_student_files(compressed_project, compression, target_location)?;
    } else {
        let mut archive = Archive::new(compressed_project, compression)?;
        if let Ok(plugin) = PluginType::from_archive(&mut archive) {
            let compressed_project = archive.into_inner();
            plugin.extract_student_files(compressed_project, compression, target_location)?;
        } else {
            log::debug!(
                "no matching language plugin found for {}, extracting naively",
                target_location.display()
            );
            archive.extract(target_location)?;
        }
    }
    Ok(())
}

fn move_dir(source: &Path, target: &Path) -> Result<(), LangsError> {
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

        if entry_path.file_name() == Some(OsStr::new(LOCK_FILE_NAME)) {
            log::info!("skipping lock file");
            file_count_copied += 1;
            progress_stage(format!(
                "Skipped moving file {file_count_copied} / {file_count_total}"
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
                "Moved file {file_count_copied} / {file_count_total}"
            ));
        } else if entry_path.is_dir() {
            log::debug!("Deleting {}", entry_path.display());
            file_util::remove_dir_empty(entry_path)?;
        }
    }

    // remove lock file if any
    file_util::remove_file(source.join(file_util::LOCK_FILE_NAME)).ok();
    file_util::remove_dir_empty(source)?;

    finish_stage("Finished moving project directory");
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

fn extract_project_overwrite(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    compression: Compression,
) -> Result<(), LangsError> {
    match compression {
        Compression::Tar => {
            let mut archive = tar::Archive::new(compressed_project);
            archive
                .unpack(target_location)
                .map_err(|e| LangsError::TarExtract(target_location.to_path_buf(), e))?;
        }
        Compression::TarZstd => {
            let decoder = zstd::Decoder::new(compressed_project).map_err(LangsError::ZstdDecode)?;
            let mut archive = tar::Archive::new(decoder);
            archive
                .unpack(target_location)
                .map_err(|e| LangsError::TarExtract(target_location.to_path_buf(), e))?;
        }
        Compression::Zip => {
            let mut archive = zip::ZipArchive::new(compressed_project)?;
            archive
                .extract(target_location)
                .map_err(|e| LangsError::ZipExtract(target_location.to_path_buf(), e))?;
        }
    }
    Ok(())
}

fn get_default_sandbox_image(path: &Path) -> Result<&'static str, LangsError> {
    let img = match PluginType::from_exercise(path)? {
        PluginType::CSharp => CSharpPlugin::DEFAULT_SANDBOX_IMAGE,
        PluginType::Make => MakePlugin::DEFAULT_SANDBOX_IMAGE,
        // the Java plugin is disabled on musl
        #[cfg(not(target_env = "musl"))]
        PluginType::Maven => MavenPlugin::DEFAULT_SANDBOX_IMAGE,
        // the Java plugin is disabled on musl
        #[cfg(not(target_env = "musl"))]
        PluginType::Ant => AntPlugin::DEFAULT_SANDBOX_IMAGE,
        PluginType::NoTests => NoTestsPlugin::DEFAULT_SANDBOX_IMAGE,
        PluginType::Python3 => Python3Plugin::DEFAULT_SANDBOX_IMAGE,
        PluginType::R => RPlugin::DEFAULT_SANDBOX_IMAGE,
    };
    Ok(img)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use mockito::Server;
    use std::io::Write;
    use tmc_testmycode_client::response::ExercisesDetails;
    use zip::write::SimpleFileOptions;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Trace)
            .with_module_level("j4rs", LevelFilter::Warn)
            .with_module_level("mockito", LevelFilter::Warn)
            .with_module_level("reqwest", LevelFilter::Warn)
            .init();
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

    fn mock_testmycode_client(server: &Server) -> tmc::TestMyCodeClient {
        let mut client = tmc::TestMyCodeClient::new(
            server.url().parse().unwrap(),
            "client".to_string(),
            "version".to_string(),
        )
        .unwrap();
        let token = tmc::Token::new(
            AccessToken::new("".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    #[test]
    fn signs_with_jwt() {
        init();

        let value = "some string";
        let secret = "some secret".as_bytes();
        let signed = sign_with_jwt(value, secret).unwrap();
        assert_eq!(
            signed,
            "eyJhbGciOiJIUzI1NiJ9.InNvbWUgc3RyaW5nIg.FfWkq8BeQRe2vlrfLbJHObFAslXqK5_V_hH2TbBqggc"
        );
    }

    #[test]
    fn gets_projects_dir() {
        init();

        let projects_dir = get_projects_dir("client").unwrap();
        assert!(projects_dir.ends_with("client"));
        let parent = projects_dir.parent().unwrap();
        assert!(parent.ends_with("tmc"));
    }

    #[test]
    fn checks_exercise_updates() {
        init();
        let mut server = Server::new();

        let details = vec![
            ExercisesDetails {
                id: 1,
                course_name: "some course".to_string(),
                exercise_name: "some exercise".to_string(),
                checksum: "new checksum".to_string(),
                hide_submission_results: false,
            },
            ExercisesDetails {
                id: 2,
                course_name: "some course".to_string(),
                exercise_name: "another exercise".to_string(),
                checksum: "old checksum".to_string(),
                hide_submission_results: false,
            },
        ];
        let mut response = HashMap::new();
        response.insert("exercises", details);
        let response = serde_json::to_string(&response).unwrap();
        let _m = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(response)
            .create();

        let projects_dir = tempfile::tempdir().unwrap();

        file_to(
            &projects_dir,
            "some course/course_config.toml",
            r#"
course = 'some course'

[exercises."some exercise"]
id = 1
checksum = 'old checksum'

[exercises."another exercise"]
id = 2
checksum = 'old checksum'
"#,
        );
        file_to(&projects_dir, "some course/some exercise/some file", "");

        let client = mock_testmycode_client(&server);
        let updates = check_exercise_updates(&client, projects_dir.path()).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(&updates[0], &1);
    }

    #[test]
    fn downloads_old_submission() {
        init();
        let mut server = Server::new();

        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        zw.start_file("src/file", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"file contents").unwrap();
        let z = zw.finish().unwrap();
        let _m = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(z.into_inner())
            .create();

        let output_dir = tempfile::tempdir().unwrap();
        let client = mock_testmycode_client(&server);

        download_old_submission(&client, 1, output_dir.path(), 2, false).unwrap();
        let s = file_util::read_file_to_string(output_dir.path().join("src/file")).unwrap();
        assert_eq!(s, "file contents");
    }

    #[test]
    fn downloads_or_updates_course_exercises() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        file_to(
            &projects_dir,
            "some course/course_config.toml",
            r#"
course = 'some course'

[exercises."on disk exercise with update and submission"]
id = 1
checksum = 'old checksum'

[exercises."on disk exercise without update"]
id = 2
checksum = 'new checksum'
"#,
        );
        file_to(
            &projects_dir,
            "some course/on disk exercise with update and submission/some file",
            "",
        );
        file_to(
            &projects_dir,
            "some course/on disk exercise without update/some file",
            "",
        );

        let client = mock_testmycode_client(&server);

        let exercises = vec![1, 2, 3];

        let mut body = HashMap::new();
        body.insert(
            "exercises",
            vec![
                ExercisesDetails {
                    id: 1,
                    checksum: "new checksum".to_string(),
                    course_name: "some course".to_string(),
                    exercise_name: "on disk exercise with update and submission".to_string(),
                    hide_submission_results: false,
                },
                ExercisesDetails {
                    id: 2,
                    checksum: "new checksum".to_string(),
                    course_name: "some course".to_string(),
                    exercise_name: "on disk exercise without update".to_string(),
                    hide_submission_results: false,
                },
                ExercisesDetails {
                    id: 3,
                    checksum: "new checksum".to_string(),
                    course_name: "another course".to_string(),
                    exercise_name: "not on disk exercise with submission".to_string(),
                    hide_submission_results: false,
                },
                ExercisesDetails {
                    id: 4,
                    checksum: "new checksum".to_string(),
                    course_name: "another course".to_string(),
                    exercise_name: "not on disk exercise without submission".to_string(),
                    hide_submission_results: false,
                },
                ExercisesDetails {
                    id: 5,
                    checksum: "new checksum".to_string(),
                    course_name: "another course".to_string(),
                    exercise_name:
                        "not on disk exercise with submission exercise hide submission result"
                            .to_string(),
                    hide_submission_results: true,
                },
            ],
        );
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/details".to_string()),
            )
            .with_body(serde_json::to_string(&body).unwrap())
            .create();

        let sub_body = vec![tmc::response::Submission {
            id: 1,
            user_id: 1,
            pretest_error: None,
            created_at: chrono::Utc::now()
                .with_timezone(&chrono::FixedOffset::east_opt(0).unwrap()),
            exercise_name: "e1".to_string(),
            course_id: 1,
            processed: true,
            all_tests_passed: true,
            points: None,
            processing_tried_at: None,
            processing_began_at: None,
            processing_completed_at: None,
            times_sent_to_sandbox: 1,
            processing_attempts_started_at: chrono::Utc::now()
                .with_timezone(&chrono::FixedOffset::east_opt(0).unwrap()),
            params_json: None,
            requires_review: false,
            requests_review: false,
            reviewed: false,
            message_for_reviewer: "".to_string(),
            newer_submission_reviewed: false,
            review_dismissed: false,
            paste_available: false,
            message_for_paste: "".to_string(),
            paste_key: None,
        }];
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/1".to_string()),
                    mockito::Matcher::Regex("submissions".to_string()),
                ]),
            )
            .with_body(serde_json::to_string(&sub_body).unwrap())
            .create();

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/2".to_string()),
                    mockito::Matcher::Regex("submissions".to_string()),
                ]),
            )
            .with_body(serde_json::to_string(&[0; 0]).unwrap())
            .create();

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/3".to_string()),
                    mockito::Matcher::Regex("submissions".to_string()),
                ]),
            )
            .with_body(serde_json::to_string(&sub_body).unwrap())
            .create();

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/4".to_string()),
                    mockito::Matcher::Regex("submissions".to_string()),
                ]),
            )
            .with_body(serde_json::to_string(&[0; 0]).unwrap())
            .create();

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/5".to_string()),
                    mockito::Matcher::Regex("submissions".to_string()),
                ]),
            )
            .with_body(serde_json::to_string(&sub_body).unwrap())
            .create();

        let mut template_zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        template_zw
            .start_file("src/student_file.py", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();
        template_zw
            .start_file(
                "src/template_only_student_file.py",
                SimpleFileOptions::default(),
            )
            .unwrap();
        template_zw.write_all(b"template").unwrap();
        template_zw
            .start_file("test/exercise_file.py", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();
        template_zw
            .start_file("setup.py", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();
        let template_z = template_zw.finish().unwrap();
        let template_z = template_z.into_inner();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/1".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(&template_z)
            .create();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/2".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(&template_z)
            .create();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/3".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(&template_z)
            .create();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/4".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(&template_z)
            .create();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises/5".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(&template_z)
            .create();

        let mut sub_zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        sub_zw
            .start_file("src/student_file.py", SimpleFileOptions::default())
            .unwrap();
        sub_zw.write_all(b"submission").unwrap();
        sub_zw
            .start_file("test/exercise_file.py", SimpleFileOptions::default())
            .unwrap();
        sub_zw.write_all(b"submission").unwrap();
        sub_zw
            .start_file(
                "test/submission_only_exercise_file.py",
                SimpleFileOptions::default(),
            )
            .unwrap();
        sub_zw.write_all(b"submission").unwrap();
        sub_zw
            .start_file("setup.py", SimpleFileOptions::default())
            .unwrap();
        sub_zw.write_all(b"submission").unwrap();
        let sub_z = sub_zw.finish().unwrap();
        let sub_z = sub_z.into_inner();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("submissions/1".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(sub_z)
            .create();

        let res =
            download_or_update_course_exercises(&client, projects_dir.path(), &exercises, false)
                .unwrap();
        let (downloaded, skipped) = match res {
            DownloadResult::Success {
                downloaded,
                skipped,
            } => (downloaded, skipped),
            other => panic!("{other:?}"),
        };

        assert_eq!(downloaded.len(), 4);
        assert_eq!(skipped.len(), 1);

        let e1 = downloaded.iter().find(|e| e.id == 1).unwrap();
        let _e2 = skipped.iter().find(|e| e.id == 2).unwrap();
        let e3 = downloaded.iter().find(|e| e.id == 3).unwrap();
        let e4 = downloaded.iter().find(|e| e.id == 4).unwrap();
        let e5 = downloaded.iter().find(|e| e.id == 5).unwrap();

        // did not download submission even though it was available because it was on disk
        let f = file_util::read_file_to_string(e1.path.join("src/student_file.py")).unwrap();
        assert_eq!(f, "template");
        assert!(e1.path.join("src/template_only_student_file.py").exists());
        let f = file_util::read_file_to_string(e1.path.join("test/exercise_file.py")).unwrap();
        assert_eq!(f, "template");

        // downloaded template, removed all student files and added all student files from submission
        let f = file_util::read_file_to_string(e3.path.join("src/student_file.py")).unwrap();
        assert_eq!(f, "submission");
        assert!(!e3.path.join("src/template_only_student_file.py").exists());
        assert!(
            !e3.path
                .join("test/submission_only_exercise_file.py")
                .exists()
        );
        let f = file_util::read_file_to_string(e3.path.join("test/exercise_file.py")).unwrap();
        assert_eq!(f, "template");

        // did not download submission because one was not available
        let f = file_util::read_file_to_string(e4.path.join("src/student_file.py")).unwrap();
        assert_eq!(f, "template");
        assert!(e4.path.join("src/template_only_student_file.py").exists());
        let f = file_util::read_file_to_string(e4.path.join("test/exercise_file.py")).unwrap();
        assert_eq!(f, "template");

        // did not download submission because exercise hides submission results, for example exam exercise
        let f = file_util::read_file_to_string(e5.path.join("src/student_file.py")).unwrap();
        assert_eq!(f, "template");
        assert!(e5.path.join("src/template_only_student_file.py").exists());
        let f = file_util::read_file_to_string(e5.path.join("test/exercise_file.py")).unwrap();
        assert_eq!(f, "template");
    }

    #[test]
    fn download_old_submission_keeps_new_exercise_files() {
        init();
        let mut server = Server::new();

        let output_dir = tempfile::tempdir().unwrap();

        // exercise template
        let mut template_zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        template_zw
            .start_file("pom.xml", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();

        template_zw
            .start_file("src/main/java/File.java", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();

        template_zw
            .start_file("src/test/java/FileTest.java", SimpleFileOptions::default())
            .unwrap();
        template_zw.write_all(b"template").unwrap();

        let template_z = template_zw.finish().unwrap();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(template_z.into_inner())
            .create();

        // submission
        let mut submission_zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        submission_zw
            .start_file("pom.xml", SimpleFileOptions::default())
            .unwrap();
        submission_zw.write_all(b"old submission").unwrap();

        submission_zw
            .start_file("src/main/java/File.java", SimpleFileOptions::default())
            .unwrap();
        submission_zw.write_all(b"old submission").unwrap();

        submission_zw
            .start_file("src/test/java/FileTest.java", SimpleFileOptions::default())
            .unwrap();
        submission_zw.write_all(b"old submission").unwrap();

        let submission_z = submission_zw.finish().unwrap();
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("submission".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(submission_z.into_inner())
            .create();

        let client = mock_testmycode_client(&server);

        download_old_submission(&client, 1, output_dir.path(), 2, false).unwrap();

        let s = file_util::read_file_to_string(output_dir.path().join("pom.xml")).unwrap();
        assert_eq!(s, "template");
        let s = file_util::read_file_to_string(output_dir.path().join("src/main/java/File.java"))
            .unwrap();
        assert_eq!(s, "old submission");
        let s =
            file_util::read_file_to_string(output_dir.path().join("src/test/java/FileTest.java"))
                .unwrap();
        assert_eq!(s, "template");
    }
}
