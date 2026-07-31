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
        Credentials, MoocAuth, MoocAuthFailure, MoocCredentials, ProjectsConfig,
        ProjectsDirTmcExercise, TmcConfig, TmcCourseConfig, list_local_mooc_course_exercises,
        list_local_tmc_course_exercises, migrate_exercise, move_projects_dir,
    },
    course_refresher::{RefreshData, RefreshExercise, refresh_course},
    data::{
        CombinedCourseData, ConfigValue, DownloadOrUpdateMoocCourseExercisesResult,
        DownloadOrUpdateTmcCourseExercisesResult, LocalExercise, LocalMoocExercise,
        LocalTmcExercise, MoocExerciseDownload, MoocOldSubmissionRestore, TmcDownloadResult,
        TmcExerciseDownload, TmcParams,
    },
    error::{LangsError, ParamError},
    submission_packaging::{PrepareSubmission, prepare_submission},
    submission_processing::prepare_solution,
};
use jwt_simple::prelude::*;
// use heim::disk;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
pub use tmc_langs_util::{FileError, file_util, notification_reporter, progress_reporter};
pub use tmc_mooc_client as mooc;
use tmc_mooc_client::MoocClient;
pub use tmc_testmycode_client as tmc;
use toml::Value as TomlValue;
use url::Url;
use uuid::Uuid;
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
/// use jwt_simple::prelude::*;
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct TestResult {
///     passed: bool,
/// }
///
/// // the secret must be at least 96 bits (12 bytes) long
/// let secret = "example secret key".as_bytes();
/// let token = tmc_langs::sign_with_jwt(TestResult { passed: true }, secret).unwrap();
///
/// // token has time-based claims (iat/exp), so verify by decoding it instead
/// // of comparing to a fixed string
/// let key = HS256Key::from_bytes(secret);
/// let claims = key.verify_token::<TestResult>(&token, None).unwrap();
/// assert!(claims.custom.passed);
/// ```
///
/// # Errors
/// Should never fail, but returns an error to be safe against changes in external libraries.
pub fn sign_with_jwt<T: Serialize>(value: T, secret: &[u8]) -> Result<String, LangsError> {
    let key = HS256Key::from_bytes(secret);
    let claims = Claims::with_custom_claims(value, Duration::from_mins(15));
    let token = key.authenticate(claims)?;
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
pub fn check_tmc_exercise_updates(
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
                .ok_or(LangsError::TmcExerciseMissingOnServer(local_exercise.id))?;
            if server_exercise.checksum != local_exercise.checksum {
                // server has an updated exercise
                updated_exercises.push(local_exercise.id);
            }
        }
    }
    Ok(updated_exercises)
}

/// Checks the server for any updates for exercises within the given projects directory.
/// Returns the ids of each exercise that can be updated.
pub fn check_mooc_exercise_updates(
    client: &mooc::MoocClient,
    auth: &MoocAuth,
    projects_dir: &Path,
) -> Result<Vec<Uuid>, LangsError> {
    log::debug!("checking exercise updates in {}", projects_dir.display());

    let mut updated_exercises = vec![];

    let config = ProjectsConfig::load(projects_dir)?;

    // One `course_exercises` request per course, not one per exercise; correlate
    // by exercise id (the config map key and the extension's identity) — the
    // stored `task_id` is the editor task's id, not a valid exercise key.
    let mut server_exercises: HashMap<Uuid, mooc::TmcExerciseSlide> = HashMap::new();
    for course_config in config.mooc_courses.values() {
        if course_config.exercises.is_empty() {
            continue;
        }
        let slides = auth.call(client, |c| c.course_exercises(course_config.course_id))?;
        for slide in slides {
            server_exercises.insert(slide.exercise_id, slide);
        }
    }

    for course_config in config.mooc_courses.values() {
        for (exercise_id, local_exercise) in &course_config.exercises {
            let server_exercise = server_exercises
                .get(exercise_id)
                .ok_or(LangsError::MoocExerciseMissingOnServer(*exercise_id))?;
            if server_exercise
                .editor_checksum()
                .map(|cs| cs != local_exercise.checksum)
                .unwrap_or_default()
            {
                // server has an updated exercise
                updated_exercises.push(*exercise_id);
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
) -> Result<TmcDownloadResult, LangsError> {
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
        return Ok(TmcDownloadResult::Failure {
            downloaded,
            skipped: to_be_skipped,
            failed,
        });
    }

    Ok(TmcDownloadResult::Success {
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

/// tmc-server hosts that may be handed a courses.mooc.fi access token.
///
/// The tmc root URL is taken from the environment, so it is not by itself
/// evidence that the host really is tmc-server. Restricting the reuse of the mooc
/// access token to this list means a redirected root URL cannot turn a tmc
/// command into a token handover.
const TMC_HOSTS_TRUSTED_WITH_MOOC_TOKEN: &[&str] = &["tmc.mooc.fi"];

/// See [`TMC_HOSTS_TRUSTED_WITH_MOOC_TOKEN`]. Loopback is allowed only under the
/// same opt-in the mooc client uses, so a mock or a locally served backend can
/// exercise the path without production ever trusting a local host implicitly.
fn tmc_host_may_receive_mooc_token(root_url: &Url) -> bool {
    let Some(host) = root_url.host_str() else {
        return false;
    };
    if TMC_HOSTS_TRUSTED_WITH_MOOC_TOKEN.contains(&host) {
        // A bearer for a second backend must not cross the network in plaintext.
        return root_url.scheme() == "https";
    }
    mooc::trust_localhost() && matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Which credential authenticates a [`tmc::TestMyCodeClient`].
///
/// New tmc-server logins no longer exist, so there are only two sources: a token
/// stored by an older version, and the courses.mooc.fi access token that
/// tmc-server accepts by introspecting it.
#[derive(Debug)]
pub enum TestMyCodeAuth {
    /// A tmc-server token from the legacy `credentials.json`.
    StoredTmc(Credentials),
    /// The courses.mooc.fi access token.
    Mooc(tmc::Token),
    /// No usable credential; only unauthenticated endpoints will work.
    Unauthenticated,
}

impl TestMyCodeAuth {
    /// The token the client was given, for reporting login status.
    pub fn token(&self) -> Option<tmc::Token> {
        match self {
            Self::StoredTmc(credentials) => Some(credentials.token()),
            Self::Mooc(token) => Some(token.clone()),
            Self::Unauthenticated => None,
        }
    }

    /// Takes the stored tmc credentials out, leaving the value unauthenticated.
    ///
    /// Used by the 401 handling, which may delete a rejected *tmc* token but must
    /// never touch the mooc credentials: tmc-server rejecting a mooc token says
    /// nothing about that token's validity at courses.mooc.fi (it may simply not
    /// be accepting them), so deleting it would log the user out of the backend
    /// that actually issued it.
    pub fn take_stored_tmc(&mut self) -> Option<Credentials> {
        match std::mem::replace(self, Self::Unauthenticated) {
            Self::StoredTmc(credentials) => Some(credentials),
            other => {
                *self = other;
                None
            }
        }
    }
}

/// Initializes a TestMyCodeClient with whichever credential authenticates it.
///
/// Precedence is deliberate: a stored tmc `credentials.json` wins while it
/// exists, so a user who logged in with a password before that flow was removed
/// keeps working until tmc-server rejects the token. The 401 handling then
/// deletes the file, and the next invocation falls through to the mooc access
/// token. The mooc token is only used when there is no stored tmc token at all,
/// so this never changes the credential under a session that still works.
///
/// `mooc_root_url` and `mooc_client_id` are needed to refresh the mooc token, the
/// same way [`init_mooc_client_with_credentials`] does.
pub fn init_testmycode_client_with_credentials(
    root_url: Url,
    client_name: &str,
    client_version: &str,
    mooc_root_url: &Url,
    mooc_client_id: &str,
) -> Result<(tmc::TestMyCodeClient, TestMyCodeAuth), LangsError> {
    let mut client = tmc::TestMyCodeClient::new(
        root_url.clone(),
        client_name.to_string(),
        client_version.to_string(),
    )?;

    if let Some(credentials) = Credentials::load(client_name)? {
        client.set_token(credentials.token(), tmc::TokenSource::Tmc);
        return Ok((client, TestMyCodeAuth::StoredTmc(credentials)));
    }

    if !tmc_host_may_receive_mooc_token(&root_url) {
        log::warn!(
            "not authenticating with {root_url} using the courses.mooc.fi access token: \
             the host is not trusted with it"
        );
        return Ok((client, TestMyCodeAuth::Unauthenticated));
    }

    let Some(mooc_credentials) =
        MoocCredentials::load_valid(client_name, mooc_root_url, mooc_client_id)?
    else {
        return Ok((client, TestMyCodeAuth::Unauthenticated));
    };
    let token = mooc_credentials.token();
    client.set_token(token.clone(), tmc::TokenSource::Mooc);
    Ok((client, TestMyCodeAuth::Mooc(token)))
}

/// Initializes a MoocClient, using and returning the stored credentials, if any.
///
/// The stored token is refreshed first if it is expired (see
/// [`MoocCredentials::load_valid`]), so the returned client carries a token that
/// is valid at call time when possible. `client_id` is the OAuth2 client id the
/// refresh grant is made with.
pub fn init_mooc_client_with_credentials(
    root_url: Url,
    client_name: &str,
    client_id: &str,
) -> Result<(mooc::MoocClient, Option<MoocCredentials>), LangsError> {
    // create client
    let mut client = mooc::MoocClient::new(root_url.clone())?;

    // set token from the credentials file if one exists, refreshing if expired
    let credentials = MoocCredentials::load_valid(client_name, &root_url, client_id)?;
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
                    .ok_or(LangsError::TmcExerciseMissingOnServer(local_exercise.id))?;
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
    auth: &MoocAuth,
    projects_dir: &Path,
) -> Result<DownloadOrUpdateMoocCourseExercisesResult, LangsError> {
    let mut projects_config = ProjectsConfig::load(projects_dir)?;
    // Snapshot the locals as OWNED data keyed by EXERCISE id (not the editor task
    // id), so `projects_config` can be mutated below to persist refreshed
    // checksums.
    struct LocalMoocExerciseInfo {
        instance_id: Uuid,
        course_directory: String,
        exercise_directory: String,
        exercise_name: String,
        checksum: String,
    }
    let locals: HashMap<Uuid, LocalMoocExerciseInfo> = projects_config
        .mooc_courses
        .iter()
        .flat_map(|(instance_id, cc)| {
            cc.exercises.iter().map(move |(id, e)| {
                (
                    *id,
                    LocalMoocExerciseInfo {
                        instance_id: *instance_id,
                        course_directory: cc.directory.clone(),
                        exercise_directory: e.directory.clone(),
                        exercise_name: e.name.clone(),
                        checksum: e.checksum.clone(),
                    },
                )
            })
        })
        .collect();

    let mut downloaded = Vec::new();
    if !locals.is_empty() {
        // One `course_exercises` request per course, not one per exercise.
        // `instance_id` and `course_id` coincide for mooc courses.
        let course_ids: HashSet<Uuid> = projects_config
            .mooc_courses
            .values()
            .map(|cc| cc.course_id)
            .collect();
        let mut server_exercises: Vec<mooc::TmcExerciseSlide> = Vec::new();
        for course_id in course_ids {
            server_exercises.extend(auth.call(client, |c| c.course_exercises(course_id))?);
        }
        for slide in server_exercises {
            let Some(local) = locals.get(&slide.exercise_id) else {
                // Not tracked locally (e.g. deleted). Nothing to update.
                continue;
            };
            // Browser exercises have no editor checksum; skip them.
            let Some(new_checksum) = slide.editor_checksum() else {
                continue;
            };
            if new_checksum == local.checksum {
                continue;
            }
            let target = ProjectsConfig::get_mooc_exercise_download_target(
                projects_dir,
                &local.course_directory,
                &local.exercise_directory,
            );
            // Download directly from the editor task's `stub_download_url`
            // (`.tar.zst`); we already have the slide.
            let Some(download_url) = slide.editor_stub_download_url() else {
                log::warn!(
                    "Skipping exercise {}: no downloadable editor task in public spec",
                    slide.exercise_id
                );
                continue;
            };
            download_and_extract_mooc_archive(client, auth, download_url, &target)?;

            // Persist the refreshed checksum, or the same update is re-reported on
            // every subsequent check. Use the slide's editor task id (the id the
            // checksum belongs to), falling back to the stored task id.
            let task_id = slide
                .editor_task_id()
                .or_else(|| {
                    projects_config
                        .mooc_courses
                        .get(&local.instance_id)
                        .and_then(|cc| cc.exercises.get(&slide.exercise_id))
                        .map(|e| e.task_id)
                })
                .unwrap_or_else(Uuid::nil);
            if let Some(course_config) = projects_config.mooc_courses.get_mut(&local.instance_id) {
                course_config.add_exercise(
                    slide.exercise_id,
                    local.exercise_name.clone(),
                    task_id,
                    new_checksum.to_string(),
                );
                course_config.save_to_projects_dir(projects_dir)?;
            }

            downloaded.push(MoocExerciseDownload {
                exercise_id: slide.exercise_id,
                path: target,
            });
        }
    }

    Ok(DownloadOrUpdateMoocCourseExercisesResult {
        downloaded,
        skipped: vec![],
        failed: None,
        not_attempted: vec![],
        stopped_for_auth: false,
    })
}

/// Flattens an error's `source()` chain into human-readable strings, most
/// specific first, for the CLI's `failed` list entries.
fn error_chain(err: &dyn std::error::Error) -> Vec<String> {
    let mut chain = vec![err.to_string()];
    let mut error = err;
    while let Some(source) = error.source() {
        chain.push(source.to_string());
        error = source;
    }
    chain
}

/// The failure shape of [`download_and_extract_mooc_archive`]: an auth failure
/// (see [`MoocAuthFailure::is_permanent`]), or a purely local one (bad URL, extraction).
enum DownloadArchiveError {
    Auth(MoocAuthFailure),
    Local(LangsError),
}

impl From<DownloadArchiveError> for LangsError {
    fn from(err: DownloadArchiveError) -> Self {
        match err {
            DownloadArchiveError::Auth(e) => e.into(),
            DownloadArchiveError::Local(e) => e,
        }
    }
}

/// Downloads a mooc exercise stub archive from `download_url` (an editor task's
/// public spec `stub_download_url`) and extracts it into `target`. The backend's
/// file-store archives are `.tar.zst`.
fn download_and_extract_mooc_archive(
    client: &MoocClient,
    auth: &MoocAuth,
    download_url: &str,
    target: &Path,
) -> Result<(), DownloadArchiveError> {
    let url = Url::parse(download_url).map_err(|err| {
        DownloadArchiveError::Local(LangsError::MoocClient(Box::new(
            mooc::MoocClientError::UrlParse(download_url.to_string(), err),
        )))
    })?;
    let data = auth
        .call(client, |c| c.download(url.clone()))
        .map_err(DownloadArchiveError::Auth)?;
    extract_project(
        Cursor::new(data),
        target,
        Compression::TarZstd,
        false,
        false,
    )
    .map_err(DownloadArchiveError::Local)?;
    Ok(())
}

/// Downloads a past mooc submission and restores it at `output_path`, overlaying
/// the submission's student files on top of a fresh exercise stub.
///
/// Mirrors the TMC [`download_old_submission`] flow minus the server-reset step
/// (mooc has no reset): a fresh stub provides the non-student template files, and
/// only the submission's student files are extracted over it. The exercise is
/// rebuilt in a temp dir and moved into `output_path`, fully replacing it (stale
/// files dropped). If `save_old_state` is set, the current state is submitted
/// first (non-blocking) so nothing the student wrote is lost.
///
/// A submission with no downloadable files (an answer made in the browser) is
/// reported as [`MoocOldSubmissionRestore::NothingToDownload`]; the archive is
/// resolved before anything else so that case leaves the local exercise — and the
/// server — untouched.
pub fn download_mooc_old_submission(
    client: &MoocClient,
    auth: &MoocAuth,
    exercise_id: Uuid,
    output_path: &Path,
    submission_id: Uuid,
    save_old_state: bool,
) -> Result<MoocOldSubmissionRestore, LangsError> {
    log::debug!(
        "downloading old mooc submission {submission_id} for exercise {exercise_id} to {}",
        output_path.display()
    );

    let archive_url = auth.call(client, |c| c.download_submission_archive_url(submission_id))?;
    let Some(archive_url) = archive_url else {
        log::debug!("submission {submission_id} has no downloadable files");
        return Ok(MoocOldSubmissionRestore::NothingToDownload);
    };

    if save_old_state {
        let temp = file_util::named_temp_file()?;
        compress_project_to(output_path, temp.path(), Compression::TarZstd, false, false)?;
        auth.call(client, |c| c.submit_exercise(exercise_id, temp.path()))?;
        log::debug!("submitted current state before downloading old submission");
    }

    let temp_dir = tempfile::tempdir().map_err(FileError::TempFile)?;
    let base = temp_dir.path();
    let stub = auth.call(client, |c| c.download_exercise(exercise_id))?;
    extract_project(Cursor::new(stub), base, Compression::TarZstd, false, false)?;
    log::debug!("extracted fresh stub to temp base");

    let url = Url::parse(&archive_url)
        .map_err(|err| Box::new(mooc::MoocClientError::UrlParse(archive_url.clone(), err)))?;
    let archive = auth.call(client, |c| c.download(url.clone()))?;
    extract_student_files(Cursor::new(archive), Compression::TarZstd, base)?;
    log::debug!("overlaid old submission student files");

    if output_path.exists() {
        file_util::remove_dir_all(output_path)?;
    }
    file_util::create_dir_all(output_path)?;
    move_dir(base, output_path)?;
    log::debug!("moved restored exercise into place");
    Ok(MoocOldSubmissionRestore::Restored)
}

/// Downloads or updates the given mooc exercises in the local projects directory.
///
/// The course context (needed for the on-disk directory and the projects config
/// key) is resolved one of two ways:
/// - if `course_id` is given (the extension always knows it from the course
///   details), only that course's exercise slides are fetched;
/// - otherwise the user's enrolled courses are scanned to locate each exercise
///   (O(courses x exercises)), since the langs API exposes no exercise -> course
///   lookup for that case.
///
/// A course's name determines its on-disk directory, and the course id doubles as
/// the projects config instance key (the langs API does not expose course
/// instance ids).
///
/// For each requested exercise:
/// - not found -> reported as failed,
/// - no editor task (e.g. a browser-only exercise) -> reported as failed,
/// - stored checksum already matches the server -> skipped,
/// - otherwise the editor task's stub archive is downloaded, extracted as
///   `.tar.zst`, and the course config is updated.
///
/// Results are keyed by the requested `exercise_id`.
pub fn download_or_update_mooc_course_exercises(
    client: &MoocClient,
    auth: &MoocAuth,
    projects_dir: &Path,
    exercise_ids: &[Uuid],
    course_id: Option<Uuid>,
) -> Result<DownloadOrUpdateMoocCourseExercisesResult, LangsError> {
    log::debug!(
        "downloading or updating {} mooc exercises in {}",
        exercise_ids.len(),
        projects_dir.display()
    );

    let mut projects_config = ProjectsConfig::load(projects_dir)?;

    // Resolve course context for each requested exercise.
    // exercise_id -> (course_id, course_name, slide)
    let requested: HashSet<Uuid> = exercise_ids.iter().copied().collect();
    let mut resolved: HashMap<Uuid, (Uuid, String, mooc::TmcExerciseSlide)> = HashMap::new();
    if !requested.is_empty() {
        match course_id {
            // The caller knows the course: fetch just that course's slides instead
            // of scanning every enrolled course.
            Some(course_id) => {
                let course = auth.call(client, |c| c.course(course_id))?;
                for slide in auth.call(client, |c| c.course_exercises(course_id))? {
                    if requested.contains(&slide.exercise_id) {
                        resolved.insert(slide.exercise_id, (course.id, course.name.clone(), slide));
                    }
                }
            }
            // No course context: resolve each exercise's course by scanning the
            // enrolled courses' exercise slides.
            None => {
                'courses: for course in auth.call(client, |c| c.courses())? {
                    for slide in auth.call(client, |c| c.course_exercises(course.id))? {
                        if requested.contains(&slide.exercise_id)
                            && !resolved.contains_key(&slide.exercise_id)
                        {
                            resolved
                                .insert(slide.exercise_id, (course.id, course.name.clone(), slide));
                            if resolved.len() == requested.len() {
                                break 'courses;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut downloaded = Vec::new();
    let mut skipped = Vec::new();
    let mut failed: Vec<(MoocExerciseDownload, Vec<String>)> = Vec::new();
    // Index the batch stopped at on a permanent auth failure, so the rest can be
    // reported as `not_attempted` instead of silently dropped.
    let mut stop_at: Option<usize> = None;

    // Report per-exercise download progress, mirroring the TMC download path.
    let total_steps = u32::try_from(exercise_ids.len())
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    progress_reporter::start_stage::<mooc::MoocClientUpdateData>(
        total_steps,
        format!("Downloading {} mooc exercises", exercise_ids.len()),
        None,
    );

    for (item_index, &exercise_id) in exercise_ids.iter().enumerate() {
        let Some((course_id, course_name, slide)) = resolved.get(&exercise_id) else {
            failed.push((
                MoocExerciseDownload {
                    exercise_id,
                    path: projects_dir.join("mooc"),
                },
                vec![format!(
                    "Exercise {exercise_id} was not found in any of the user's enrolled courses"
                )],
            ));
            continue;
        };

        let editor_task = slide.tasks.iter().find(|task| {
            task.public_spec
                .as_ref()
                .and_then(|ps| ps.editor_stub_download_url())
                .is_some()
        });
        let Some(task) = editor_task else {
            failed.push((
                MoocExerciseDownload {
                    exercise_id,
                    path: projects_dir.join("mooc"),
                },
                vec![format!(
                    "Exercise {exercise_id} has no downloadable editor task \
                     (browser exercises have no project archive)"
                )],
            ));
            continue;
        };
        let public_spec = task
            .public_spec
            .as_ref()
            .expect("editor task guarantees a public spec");
        let download_url = public_spec
            .editor_stub_download_url()
            .expect("editor task guarantees a stub download url")
            .to_string();
        let task_id = task.task_id;
        let checksum = task.checksum.clone().unwrap_or_default();
        let exercise_name = slide.exercise_name.clone();

        let course_config = projects_config.get_or_init_mooc_course_config(
            *course_id,
            *course_id,
            course_name.clone(),
        );

        // resolve the target directory (reuse the existing one if already downloaded)
        let exercise_directory = course_config
            .exercises
            .get(&exercise_id)
            .map(|e| e.directory.clone())
            .unwrap_or_else(|| config::simple_kebab_case(&exercise_name));
        let target = ProjectsConfig::get_mooc_exercise_download_target(
            projects_dir,
            &course_config.directory,
            &exercise_directory,
        );

        // skip if the stored checksum already matches
        if let Some(existing) = course_config.exercises.get(&exercise_id) {
            if existing.checksum == checksum {
                log::info!("Skipping exercise {exercise_id} due to identical checksum");
                skipped.push(MoocExerciseDownload {
                    exercise_id,
                    path: target,
                });
                continue;
            }
        }

        progress_reporter::progress_stage::<mooc::MoocClientUpdateData>(
            format!(
                "Downloading exercise {exercise_id} to '{}'",
                target.display()
            ),
            Some(mooc::MoocClientUpdateData::ExerciseDownload {
                id: exercise_id,
                path: target.clone(),
            }),
        );

        match download_and_extract_mooc_archive(client, auth, &download_url, &target) {
            Ok(()) => {
                course_config.add_exercise(exercise_id, exercise_name, task_id, checksum);
                course_config.save_to_projects_dir(projects_dir)?;
                downloaded.push(MoocExerciseDownload {
                    exercise_id,
                    path: target,
                });
            }
            // Every remaining item would fail identically: record this one as failed,
            // stop iterating, and report the rest as `not_attempted`.
            Err(DownloadArchiveError::Auth(auth_err)) if auth_err.is_permanent() => {
                log::error!(
                    "mooc auth permanently failed while downloading exercise {exercise_id}, \
                     stopping the batch: {auth_err}"
                );
                failed.push((
                    MoocExerciseDownload {
                        exercise_id,
                        path: target,
                    },
                    error_chain(&auth_err),
                ));
                stop_at = Some(item_index);
                break;
            }
            Err(err) => {
                let err: LangsError = err.into();
                failed.push((
                    MoocExerciseDownload {
                        exercise_id,
                        path: target,
                    },
                    error_chain(&err),
                ));
            }
        }
    }

    progress_reporter::finish_stage::<mooc::MoocClientUpdateData>(
        format!("Finished downloading {} mooc exercises", exercise_ids.len()),
        None,
    );

    // Everything after the stop point was never attempted.
    let not_attempted = match stop_at {
        Some(stop_at) => exercise_ids[stop_at + 1..]
            .iter()
            .map(|&exercise_id| MoocExerciseDownload {
                exercise_id,
                path: projects_dir.join("mooc"),
            })
            .collect(),
        None => Vec::new(),
    };

    Ok(DownloadOrUpdateMoocCourseExercisesResult {
        downloaded,
        skipped,
        failed: if failed.is_empty() {
            None
        } else {
            Some(failed)
        },
        not_attempted,
        stopped_for_auth: stop_at.is_some(),
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

/// Resets a mooc exercise: mirrors TMC's [`reset`], optionally submitting the
/// current state first, then replacing the directory with a freshly extracted
/// stub.
///
/// Never leaves `exercise_path` half-written: the stub is fetched and extracted
/// into a staging sibling *before* the original is touched, so a download or
/// extraction failure leaves it intact. Only once extraction fully succeeds is
/// the old directory moved aside and the staged one swapped in (a same-filesystem
/// rename); the old copy is restored if that swap fails.
pub fn reset_mooc_exercise(
    client: &MoocClient,
    auth: &MoocAuth,
    exercise_id: Uuid,
    exercise_path: &Path,
    save_old_state: bool,
) -> Result<(), LangsError> {
    log::debug!(
        "resetting mooc exercise {exercise_id} at {}",
        exercise_path.display()
    );

    if save_old_state {
        // submit the current state before resetting
        let temp = file_util::named_temp_file()?;
        compress_project_to(
            exercise_path,
            temp.path(),
            Compression::TarZstd,
            false,
            false,
        )?;
        auth.call(client, |c| c.submit_exercise(exercise_id, temp.path()))?;
        log::debug!("submitted current state before resetting exercise");
    }

    // Fetch before touching the directory: a download failure must not wipe
    // existing work.
    let stub = auth.call(client, |c| c.download_exercise(exercise_id))?;

    // Staging dir is a sibling of `exercise_path` (same filesystem, so the
    // swap-in below is a rename, not a cross-device copy). A failed/partial
    // extraction never touches the original; on early return the `TempDir`
    // guard removes the staging dir.
    let parent = match exercise_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    file_util::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".tmc-reset-")
        .tempdir_in(parent)
        .map_err(FileError::TempFile)?;
    extract_project(
        Cursor::new(stub),
        staging.path(),
        Compression::TarZstd,
        false,
        false,
    )?;

    // Extraction succeeded; only rename/remove ops remain, so the swap can't
    // leave a half-written directory. Disarm the `TempDir` guard: it's about to
    // be moved into place, not dropped.
    let staging_path = staging.keep();

    if exercise_path.exists() {
        // Move the old dir aside instead of deleting, so it can be restored if the
        // rename-in fails. Reserve a unique name via tempdir, then free it so the
        // rename target doesn't exist (required on Windows).
        let backup_path = tempfile::Builder::new()
            .prefix(".tmc-reset-old-")
            .tempdir_in(parent)
            .map_err(FileError::TempFile)?
            .keep();
        file_util::remove_dir_all(&backup_path)?;
        file_util::rename(exercise_path, &backup_path)?;

        match file_util::rename(&staging_path, exercise_path) {
            Ok(()) => {
                // The fresh copy is in place; drop the old one.
                file_util::remove_dir_all(&backup_path)?;
            }
            Err(err) => {
                // Roll back: restore the original and clean up the stage, so the
                // failure is a no-op.
                let _ = file_util::rename(&backup_path, exercise_path);
                let _ = file_util::remove_dir_all(&staging_path);
                return Err(err.into());
            }
        }
    } else {
        file_util::rename(&staging_path, exercise_path)?;
    }
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
    use oauth2::{AccessToken, EmptyExtraTokenFields, TokenResponse, basic::BasicTokenType};
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
        client.set_token(token, tmc::TokenSource::Tmc);
        client
    }

    fn mock_mooc_client(server: &Server) -> mooc::MoocClient {
        let mut client = mooc::MoocClient::new(server.url().parse().unwrap()).unwrap();
        let token = mooc::api::Token::new(
            AccessToken::new("".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    /// A `MoocAuth` pointed at the mock server. None of these tests exercise
    /// a 401/refresh, so the client id and name are arbitrary.
    fn mock_mooc_auth(server: &Server) -> MoocAuth {
        MoocAuth::new("test", server.url().parse().unwrap(), "test-client")
    }

    /// Builds a `.tar.zst` archive from the given (relative path, contents) pairs.
    fn make_tar_zst(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            for (path, contents) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append_data(&mut header, path, *contents).unwrap();
            }
            builder.finish().unwrap();
        }
        zstd::encode_all(std::io::Cursor::new(tar_buf), 0).unwrap()
    }

    /// Writes a mooc course config with a single exercise and creates the exercise
    /// directory so the load-time maintenance pass doesn't prune it. Returns
    /// (instance_id, exercise_id, task_id).
    fn write_mooc_course_config(
        projects_dir: &std::path::Path,
        local_checksum: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let instance_id = Uuid::new_v4();
        let course_id = Uuid::new_v4();
        let exercise_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        file_to(
            projects_dir,
            "mooc/my-course/course_config.toml",
            format!(
                r#"
course_id = "{course_id}"
instance_id = "{instance_id}"
course = "My Course"
directory = "my-course"

[exercises."{exercise_id}"]
name = "Exercise 1"
task_id = "{task_id}"
checksum = "{local_checksum}"
directory = "ex-1"
"#
            ),
        );
        // the exercise dir must exist or the loader prunes it from the config
        std::fs::create_dir_all(projects_dir.join("mooc/my-course/ex-1")).unwrap();
        (instance_id, exercise_id, task_id)
    }

    /// JSON for a single exercise slide carrying one editor task with a public
    /// spec that points at `stub_download_url`.
    fn editor_slide_json(
        exercise_id: Uuid,
        task_id: Uuid,
        checksum: &str,
        stub_download_url: &str,
    ) -> String {
        serde_json::json!({
            "slide_id": Uuid::new_v4(),
            "exercise_id": exercise_id,
            "course_id": Uuid::new_v4(),
            "exercise_name": "Exercise 1",
            "exercise_order_number": 0,
            "tasks": [{
                "task_id": task_id,
                "order_number": 0,
                "assignment": [],
                "public_spec": {
                    "type": "editor",
                    "archive_name": "stub.tar.zst",
                    "stub_download_url": stub_download_url,
                    "student_file_paths": ["src/main.py"],
                    "checksum": checksum,
                },
                "model_solution_spec": null,
                "exercise_service_slug": "tmc"
            }],
        })
        .to_string()
    }

    #[test]
    fn signs_with_jwt() {
        init();

        let value = serde_json::json!({
                "some key": "some value"
        });
        let secret = "some secret some secret some secret some secret some secret some secret some secret some secret".as_bytes();
        let signed = sign_with_jwt(&value, secret).unwrap();
        let key = HS256Key::from_bytes(secret);
        let claims = key
            .verify_token::<serde_json::Value>(&signed, None)
            .unwrap();
        assert_eq!(claims.custom, value);
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
            "tmc/some course/course_config.toml",
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
        file_to(&projects_dir, "tmc/some course/some exercise/some file", "");

        let client = mock_testmycode_client(&server);
        let updates = check_tmc_exercise_updates(&client, projects_dir.path()).unwrap();
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
            "tmc/some course/course_config.toml",
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
            "tmc/some course/on disk exercise with update and submission/some file",
            "",
        );
        file_to(
            &projects_dir,
            "tmc/some course/on disk exercise without update/some file",
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
            TmcDownloadResult::Success {
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

    #[test]
    fn checks_mooc_exercise_updates() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_instance_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");

        // Update check batches via the COURSE-exercises route, not one request per
        // exercise. Only that route is mocked, so a per-exercise regression fails.
        let _m = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_body(format!(
                "[{}]",
                editor_slide_json(
                    exercise_id,
                    task_id,
                    "new checksum",
                    "http://example.com/stub.tar.zst",
                )
            ))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();
        // the update list is keyed by EXERCISE id, the identity the extension addresses
        assert_eq!(updates, vec![exercise_id]);
    }

    #[test]
    fn checks_mooc_exercise_updates_no_change() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_instance_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "same checksum");

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_body(format!(
                "[{}]",
                editor_slide_json(
                    exercise_id,
                    task_id,
                    "same checksum",
                    "http://example.com/stub.tar.zst",
                )
            ))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn updates_mooc_exercises_extracts_tar_zst() {
        // Regression test: the archive comes from the public spec's
        // `stub_download_url` (no `exercises/{id}/download` route ever existed),
        // and it is `.tar.zst`, so extraction must use `TarZstd`, not `Zip`.
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_instance_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");

        let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
        let _slide = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_body(format!(
                "[{}]",
                editor_slide_json(exercise_id, task_id, "new checksum", &stub_download_url)
            ))
            .create();
        let archive = make_tar_zst(&[("src/main.py", b"print('updated')")]);
        let _archive_mock = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(result.downloaded.len(), 1);
        // results are keyed by the exercise id, not the editor task id
        assert_eq!(result.downloaded[0].exercise_id, exercise_id);
        let extracted = projects_dir.path().join("mooc/my-course/ex-1/src/main.py");
        let contents = file_util::read_file_to_string(&extracted).unwrap();
        assert_eq!(contents, "print('updated')");
    }

    #[test]
    fn update_mooc_exercises_persists_refreshed_checksum() {
        // A re-downloaded exercise's new checksum must be persisted, or the same
        // update is re-reported on every subsequent check. A second check must
        // see none.
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_instance_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");

        let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
        // The course-exercises route reports the new checksum on every request.
        let _slide = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_body(format!(
                "[{}]",
                editor_slide_json(exercise_id, task_id, "new checksum", &stub_download_url)
            ))
            .create();
        let archive = make_tar_zst(&[("src/main.py", b"print('updated')")]);
        let _archive_mock = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);

        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();
        assert_eq!(result.downloaded.len(), 1);

        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();
        assert!(
            updates.is_empty(),
            "update re-reported after the refreshed checksum should have been persisted: {updates:?}"
        );
    }

    #[test]
    fn resets_mooc_exercise_over_local_dir() {
        // Reset replaces the whole directory, not just overlays the archive:
        // leftover.txt (absent from the fresh stub) must be gone too.
        init();
        let mut server = Server::new();
        let exercise_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let exercise_dir = tempfile::tempdir().unwrap();
        // seed stale files the reset must clear
        file_to(&exercise_dir, "src/main.py", b"stale student code");
        file_to(&exercise_dir, "leftover.txt", b"should be gone");

        let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
        let _slide = server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(editor_slide_json(
                exercise_id,
                task_id,
                "checksum",
                &stub_download_url,
            ))
            .create();
        let archive = make_tar_zst(&[("src/main.py", b"print('fresh stub')")]);
        let _archive = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        reset_mooc_exercise(&client, &auth, exercise_id, exercise_dir.path(), false).unwrap();

        let main = file_util::read_file_to_string(exercise_dir.path().join("src/main.py")).unwrap();
        assert_eq!(main, "print('fresh stub')");
        assert!(!exercise_dir.path().join("leftover.txt").exists());
    }

    #[test]
    fn reset_mooc_exercise_preserves_dir_on_extraction_failure() {
        // Extraction is staged and swapped in only on success; a failure must
        // leave the original dir untouched, never empty or half-written.
        init();
        let mut server = Server::new();
        let exercise_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        // Dedicated parent dir so staging-sibling cleanup can be asserted without
        // racing other tests under the shared system temp dir.
        let parent = tempfile::tempdir().unwrap();
        let exercise_path = parent.path().join("exercise");
        // pre-existing student work that must survive a failed reset
        file_to(&exercise_path, "src/main.py", b"student code");
        file_to(&exercise_path, "notes.txt", b"my notes");

        let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
        let _slide = server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(editor_slide_json(
                exercise_id,
                task_id,
                "checksum",
                &stub_download_url,
            ))
            .create();
        // corrupt archive: extraction must fail
        let _archive = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(b"not a valid tar.zst archive")
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = reset_mooc_exercise(&client, &auth, exercise_id, &exercise_path, false);
        assert!(
            result.is_err(),
            "a corrupt stub archive must make the reset fail"
        );

        assert_eq!(
            file_util::read_file_to_string(exercise_path.join("src/main.py")).unwrap(),
            "student code"
        );
        assert_eq!(
            file_util::read_file_to_string(exercise_path.join("notes.txt")).unwrap(),
            "my notes"
        );
        let leftovers = std::fs::read_dir(parent.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != std::ffi::OsStr::new("exercise"))
            .count();
        assert_eq!(
            leftovers, 0,
            "staging/backup directories must be cleaned up, leaving only the exercise"
        );
    }

    #[test]
    fn downloads_or_updates_mooc_course_exercises() {
        // Bulk download-or-update: a changed exercise is downloaded, an unchanged
        // one skipped, and a browser-only and an unknown exercise reported failed.
        // Course context is resolved by scanning enrolled courses (no exercise ->
        // course endpoint).
        init();
        let mut server = Server::new();

        let course_id = Uuid::new_v4();
        let ex_skip = Uuid::new_v4();
        let task_skip = Uuid::new_v4();
        let ex_new = Uuid::new_v4();
        let task_new = Uuid::new_v4();
        let ex_browser = Uuid::new_v4();
        let task_browser = Uuid::new_v4();
        let ex_missing = Uuid::new_v4();

        let projects_dir = tempfile::tempdir().unwrap();
        // pre-seed a course config so the "skip" exercise has a stored checksum
        file_to(
            &projects_dir,
            "mooc/course/course_config.toml",
            format!(
                r#"
course_id = "{course_id}"
instance_id = "{course_id}"
course = "Course"
directory = "course"

[exercises."{ex_skip}"]
name = "Skip Me"
task_id = "{task_skip}"
checksum = "same checksum"
directory = "skip-me"
"#
            ),
        );
        // the exercise dir must exist or the loader prunes it from the config
        std::fs::create_dir_all(projects_dir.path().join("mooc/course/skip-me")).unwrap();

        let stub_url = format!("{}/files/new.tar.zst", server.url());
        server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .with_body(
                serde_json::json!([{
                    "id": course_id,
                    "slug": "course",
                    "name": "Course",
                    "description": null,
                    "organization_name": "org",
                }])
                .to_string(),
            )
            .create();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
            )
            .with_body(
                serde_json::json!([
                    {
                        "slide_id": Uuid::new_v4(),
                        "exercise_id": ex_skip,
                        "course_id": Uuid::new_v4(),
                        "exercise_name": "Skip Me",
                        "exercise_order_number": 0,
                        "tasks": [{
                            "task_id": task_skip,
                            "order_number": 0,
                            "assignment": [],
                            "public_spec": {
                                "type": "editor",
                                "archive_name": "s.tar.zst",
                                "stub_download_url": stub_url,
                                "student_file_paths": ["src/main.py"],
                                "checksum": "same checksum"
                            },
                            "model_solution_spec": null,
                            "exercise_service_slug": "tmc"
                        }],
                    },
                    {
                        "slide_id": Uuid::new_v4(),
                        "exercise_id": ex_new,
                        "course_id": Uuid::new_v4(),
                        "exercise_name": "New Exercise",
                        "exercise_order_number": 1,
                        "tasks": [{
                            "task_id": task_new,
                            "order_number": 0,
                            "assignment": [],
                            "public_spec": {
                                "type": "editor",
                                "archive_name": "n.tar.zst",
                                "stub_download_url": stub_url,
                                "student_file_paths": ["src/main.py"],
                                "checksum": "new checksum"
                            },
                            "model_solution_spec": null,
                            "exercise_service_slug": "tmc"
                        }],
                    },
                    {
                        "slide_id": Uuid::new_v4(),
                        "exercise_id": ex_browser,
                        "course_id": Uuid::new_v4(),
                        "exercise_name": "Browser Exercise",
                        "exercise_order_number": 2,
                        "tasks": [{
                            "task_id": task_browser,
                            "order_number": 0,
                            "assignment": [],
                            "public_spec": {
                                "type": "browser",
                                "archive_name": "b.tar.zst",
                                "stub_download_url": stub_url,
                                "student_file_paths": [],
                                "checksum": "bsum",
                                "browser_test": { "runtime": "python", "script": "" }
                            },
                            "model_solution_spec": null,
                            "exercise_service_slug": "tmc"
                        }],
                    }
                ])
                .to_string(),
            )
            .create();
        let archive = make_tar_zst(&[("src/main.py", b"print('new')")]);
        server
            .mock("GET", "/files/new.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        // course_id = None exercises the enrolled-course scan resolution path
        let result = download_or_update_mooc_course_exercises(
            &client,
            &auth,
            projects_dir.path(),
            &[ex_skip, ex_new, ex_browser, ex_missing],
            None,
        )
        .unwrap();

        assert_eq!(result.downloaded.len(), 1, "one exercise downloaded");
        // results are keyed by the requested exercise id, not the editor task id
        assert_eq!(result.downloaded[0].exercise_id, ex_new);
        assert_eq!(result.skipped.len(), 1, "one exercise skipped");
        assert_eq!(result.skipped[0].exercise_id, ex_skip);
        let failed = result.failed.expect("two failures");
        assert_eq!(failed.len(), 2, "browser + missing exercises fail");
        // the failure entries are keyed by the requested exercise ids
        let failed_ids = failed
            .iter()
            .map(|(d, _)| d.exercise_id)
            .collect::<HashSet<_>>();
        assert!(failed_ids.contains(&ex_browser));
        assert!(failed_ids.contains(&ex_missing));

        // the new exercise was extracted under the resolved course directory
        let extracted = projects_dir
            .path()
            .join("mooc/course/new-exercise/src/main.py");
        assert_eq!(
            file_util::read_file_to_string(&extracted).unwrap(),
            "print('new')"
        );

        // and it was persisted to the course config
        let reloaded = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert!(reloaded.get_mooc_exercise(course_id, ex_new).is_some());
    }

    #[test]
    fn download_or_update_with_course_id_skips_enrolled_course_scan() {
        // With a course id, only that course's slides are fetched. The all-courses
        // scan (`GET courses`) is NOT mocked, so a regression to it fails here.
        init();
        let mut server = Server::new();

        let course_id = Uuid::new_v4();
        let ex_new = Uuid::new_v4();
        let task_new = Uuid::new_v4();

        let projects_dir = tempfile::tempdir().unwrap();
        let stub_url = format!("{}/files/new.tar.zst", server.url());

        // single-course lookup (for the course name/directory)
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "id": course_id, "slug": "course", "name": "Course",
                    "description": null, "organization_name": "org",
                })
                .to_string(),
            )
            .expect_at_least(1)
            .create();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
            )
            .with_body(
                serde_json::json!([{
                    "slide_id": Uuid::new_v4(), "exercise_id": ex_new,
                    "course_id": course_id,
                    "exercise_name": "New Exercise", "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": task_new, "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "editor", "archive_name": "n.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": ["src/main.py"], "checksum": "new checksum"
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                }])
                .to_string(),
            )
            .expect_at_least(1)
            .create();
        server
            .mock("GET", "/files/new.tar.zst")
            .with_body(make_tar_zst(&[("src/main.py", b"print('new')")]))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = download_or_update_mooc_course_exercises(
            &client,
            &auth,
            projects_dir.path(),
            &[ex_new],
            Some(course_id),
        )
        .unwrap();

        assert_eq!(result.downloaded.len(), 1);
        assert_eq!(result.downloaded[0].exercise_id, ex_new);
        let reloaded = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert!(reloaded.get_mooc_exercise(course_id, ex_new).is_some());
    }
    /// Serializes the env-dependent auth tests with the crate's other
    /// `TMC_LANGS_CONFIG_DIR` users.
    fn auth_env(config_dir: &Path, trust_localhost: bool) -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::config::env_lock();
        // SAFETY: every read/write of these vars in the crate's tests is
        // serialized by `config::env_lock`.
        unsafe {
            std::env::set_var(TMC_LANGS_CONFIG_DIR_VAR, config_dir);
            if trust_localhost {
                std::env::set_var(mooc::TRUST_LOCALHOST_VAR, "1");
            } else {
                std::env::remove_var(mooc::TRUST_LOCALHOST_VAR);
            }
        }
        guard
    }

    fn token(access: &str) -> tmc::Token {
        let mut token = tmc::Token::new(
            AccessToken::new(access.to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        token.set_expires_in(Some(&std::time::Duration::from_secs(3600)));
        token
    }

    /// Writes the legacy tmc `credentials.json` the way a version that still had
    /// a password login would have.
    fn write_stored_tmc_credentials(client_name: &str, access: &str) {
        let path = crate::config::get_tmc_dir(client_name)
            .unwrap()
            .join("credentials.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&token(access)).unwrap()).unwrap();
    }

    fn init_tmc_client(
        tmc_root: &str,
        client_name: &str,
    ) -> (tmc::TestMyCodeClient, TestMyCodeAuth) {
        init_testmycode_client_with_credentials(
            tmc_root.parse().unwrap(),
            client_name,
            "version",
            // Unused unless a refresh is needed, and the seeded tokens are valid.
            &"http://localhost:1/".parse().unwrap(),
            "test-client",
        )
        .unwrap()
    }

    #[test]
    fn a_stored_tmc_credential_takes_precedence_over_the_mooc_token() {
        let config_dir = tempfile::tempdir().unwrap();
        let _env = auth_env(config_dir.path(), true);

        write_stored_tmc_credentials("prec-tmc-wins", "stored-tmc-token");
        MoocCredentials::save("prec-tmc-wins", token("mooc-token")).unwrap();

        let (_client, auth) = init_tmc_client("http://localhost:4001/", "prec-tmc-wins");
        assert!(
            matches!(auth, TestMyCodeAuth::StoredTmc(_)),
            "a still-present tmc credential must keep working, got {auth:?}"
        );
        assert_eq!(
            auth.token().unwrap().access_token().secret(),
            "stored-tmc-token"
        );
    }

    #[test]
    fn the_mooc_token_is_used_when_there_is_no_stored_tmc_credential() {
        let config_dir = tempfile::tempdir().unwrap();
        let _env = auth_env(config_dir.path(), true);

        MoocCredentials::save("prec-mooc-only", token("mooc-token")).unwrap();

        let (_client, auth) = init_tmc_client("http://localhost:4001/", "prec-mooc-only");
        assert!(
            matches!(auth, TestMyCodeAuth::Mooc(_)),
            "the mooc token should authenticate tmc commands, got {auth:?}"
        );
        assert_eq!(auth.token().unwrap().access_token().secret(), "mooc-token");
    }

    #[test]
    fn no_credential_at_all_leaves_the_client_unauthenticated() {
        let config_dir = tempfile::tempdir().unwrap();
        let _env = auth_env(config_dir.path(), true);

        let (_client, auth) = init_tmc_client("http://localhost:4001/", "prec-none");
        assert!(matches!(auth, TestMyCodeAuth::Unauthenticated));
        assert!(auth.token().is_none());
    }

    #[test]
    fn the_mooc_token_is_withheld_from_an_untrusted_tmc_host() {
        let config_dir = tempfile::tempdir().unwrap();
        let _env = auth_env(config_dir.path(), true);

        MoocCredentials::save("prec-untrusted", token("mooc-token")).unwrap();

        // The tmc root URL comes from the environment, so pointing it elsewhere
        // must not hand the mooc access token to that host.
        for root in [
            "https://attacker.example/",
            "https://tmc.mooc.fi.attacker.example/",
            "http://tmc.mooc.fi/",
        ] {
            let (_client, auth) = init_tmc_client(root, "prec-untrusted");
            assert!(
                matches!(auth, TestMyCodeAuth::Unauthenticated),
                "the mooc token must not be used against {root}, got {auth:?}"
            );
        }
        // And the credentials themselves are untouched.
        assert!(MoocCredentials::load("prec-untrusted").unwrap().is_some());
    }

    #[test]
    fn loopback_is_trusted_with_the_mooc_token_only_under_the_opt_in() {
        let config_dir = tempfile::tempdir().unwrap();

        {
            let _env = auth_env(config_dir.path(), false);
            MoocCredentials::save("prec-loopback", token("mooc-token")).unwrap();
            let (_client, auth) = init_tmc_client("http://localhost:4001/", "prec-loopback");
            assert!(
                matches!(auth, TestMyCodeAuth::Unauthenticated),
                "production must not implicitly trust a local host, got {auth:?}"
            );
        }
        {
            let _env = auth_env(config_dir.path(), true);
            let (_client, auth) = init_tmc_client("http://localhost:4001/", "prec-loopback");
            assert!(matches!(auth, TestMyCodeAuth::Mooc(_)));
        }
    }

    #[test]
    fn production_tmc_over_https_is_trusted_with_the_mooc_token() {
        let _env = crate::config::env_lock();
        assert!(tmc_host_may_receive_mooc_token(
            &"https://tmc.mooc.fi/".parse().unwrap()
        ));
        // Never in plaintext, and never a host that merely looks like it.
        assert!(!tmc_host_may_receive_mooc_token(
            &"http://tmc.mooc.fi/".parse().unwrap()
        ));
        assert!(!tmc_host_may_receive_mooc_token(
            &"https://evil.tmc.mooc.fi/".parse().unwrap()
        ));
        assert!(!tmc_host_may_receive_mooc_token(
            &"file:///etc/passwd".parse().unwrap()
        ));
    }

    #[test]
    fn the_401_handler_can_only_delete_a_stored_tmc_credential() {
        // tmc-server rejecting a mooc token says nothing about that token's
        // validity at courses.mooc.fi, so the 401 path must find nothing to
        // delete.
        let mut auth = TestMyCodeAuth::Mooc(token("mooc-token"));
        assert!(auth.take_stored_tmc().is_none());
        assert!(
            auth.token().is_some(),
            "the mooc token must survive the attempt"
        );

        let mut auth = TestMyCodeAuth::Unauthenticated;
        assert!(auth.take_stored_tmc().is_none());
    }
}
