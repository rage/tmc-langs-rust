#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! The main tmc-langs library. Provides a convenient API to all of the functionality provided by the tmc-langs project.

mod config;
mod course_refresher;
mod data;
mod error;
mod submission_packaging;
mod submission_processing;
#[cfg(test)]
mod test_util;

use crate::{
    config::MoocCourseConfig,
    data::{DownloadTarget, DownloadTargetKind},
};
pub use crate::{
    config::{
        Credentials, MoocAuth, MoocAuthFailure, MoocCredentials, ProjectsConfig,
        ProjectsDirTmcExercise, TmcConfig, TmcCourseConfig, list_local_exercises,
        list_local_mooc_course_exercises, list_local_tmc_course_exercises, migrate_exercise,
        move_projects_dir, restore_projects, set_projects_aside,
    },
    course_refresher::{
        RefreshData, RefreshExercise, RepositoryExercise, prepare_exercise_repository,
        refresh_course,
    },
    data::{
        CombinedCourseData, CombinedMoocCourseData, ConfigValue,
        DownloadOrUpdateMoocCourseExercisesResult, DownloadOrUpdateTmcCourseExercisesResult,
        LocalExercise, LocalMoocExercise, LocalTmcExercise, MoocExerciseDownload,
        MoocOldSubmissionRestore, TmcDownloadFailure, TmcDownloadResult, TmcExerciseDownload,
        TmcParams,
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
    ffi::OsString,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
};
use tmc_langs_framework::Archive;
pub use tmc_langs_framework::{
    CommandError, Compression, ExerciseDesc, ExercisePackagingConfiguration, Language,
    LanguagePlugin, PythonVer, RunResult, RunStatus, StyleValidationError, StyleValidationResult,
    StyleValidationStrategy, TestDesc, TestResult, TmcError, TmcProjectYml,
};
use tmc_langs_plugins::{
    CSharpPlugin, MakePlugin, NoTestsPlugin, Plugin, PluginType, Python3Plugin, RPlugin,
};
// the Java plugin is disabled on musl
#[cfg(not(target_env = "musl"))]
use tmc_langs_plugins::{AntPlugin, MavenPlugin};
pub use tmc_langs_util::{FileError, file_util, notification_reporter, progress_reporter};
use tmc_mooc_client::MoocClient;
pub use tmc_mooc_client::{self as mooc, MoocDownloadFailure};
pub use tmc_testmycode_client as tmc;
use toml::Value as TomlValue;
use url::Url;
use uuid::Uuid;
use walkdir::WalkDir;

const TMC_LANGS_CONFIG_DIR_VAR: &str = "TMC_LANGS_CONFIG_DIR";

/// What [`check_tmc_exercise_updates`] found among the local TMC exercises.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExerciseUpdates {
    /// Exercises whose server version differs from the local copy.
    pub updated: Vec<u32>,
    /// Exercises the server no longer has. Their local copies are left in place and are never updated.
    pub removed: Vec<u32>,
}

/// What [`check_mooc_exercise_updates`] found among the local mooc exercises; like
/// [`ExerciseUpdates`] but keyed by exercise id.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct MoocExerciseUpdates {
    /// Exercises whose server version differs from the local copy.
    pub updated: Vec<Uuid>,
    /// Exercises their course no longer has. Their local copies are left in place and are never updated.
    /// Exercises of a chapter that has not opened yet are absent from the course but not removed.
    pub removed: Vec<Uuid>,
    /// Courses on disk that courses.mooc.fi refused to serve. `updated` and `removed` say nothing
    /// about their exercises.
    pub unchecked_courses: Vec<UncheckedMoocCourse>,
}

/// A course [`check_mooc_exercise_updates`] could not check.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct UncheckedMoocCourse {
    pub course_id: Uuid,
    pub kind: CourseRefusalKind,
}

/// How courses.mooc.fi refused one course. The values are the CLI error kinds the same response
/// fails a single-course command with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum CourseRefusalKind {
    /// A 403 `insufficient_scope`: the session lacks the `exercise-services` scope.
    InsufficientScope,
    /// Any other 403: the user may not view the course, e.g. a draft.
    Forbidden,
    /// A 404: the course does not exist, or not for this user.
    NotFound,
}

impl CourseRefusalKind {
    /// The refusal `err` is, if it concerns only the course asked for. A permanent auth failure,
    /// an obsolete client and every other status concern the whole session.
    fn of(err: &MoocAuthFailure) -> Option<Self> {
        let MoocAuthFailure::Other(err) = err else {
            return None;
        };
        let mooc::MoocClientError::HttpError {
            status,
            obsolete_client: false,
            message_key,
            ..
        } = err.as_ref()
        else {
            return None;
        };
        match status.as_u16() {
            403 if message_key.as_deref() == Some(mooc::INSUFFICIENT_SCOPE_MESSAGE_KEY) => {
                Some(Self::InsufficientScope)
            }
            403 => Some(Self::Forbidden),
            404 => Some(Self::NotFound),
            _ => None,
        }
    }
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
///
/// Fails with [`LangsError::ProjectsDirUnreachable`] when the folder is missing, e.g. on a
/// drive that is not connected.
pub fn get_projects_dir(client_name: &str) -> Result<PathBuf, LangsError> {
    let projects_dir = TmcConfig::load(client_name)?.projects_dir;
    if !projects_dir.is_dir() {
        return Err(LangsError::ProjectsDirUnreachable(projects_dir));
    }
    Ok(projects_dir)
}

/// Checks the server for updates to the TMC exercises in the given projects directory.
pub fn check_tmc_exercise_updates(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
) -> Result<ExerciseUpdates, LangsError> {
    log::debug!("checking exercise updates in {}", projects_dir.display());

    let mut updates = ExerciseUpdates::default();

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
            let Some(server_exercise) = server_exercises.get(&local_exercise.id) else {
                updates.removed.push(local_exercise.id);
                continue;
            };
            if server_exercise.checksum != local_exercise.checksum {
                updates.updated.push(local_exercise.id);
            }
        }
    }
    Ok(updates)
}

/// Checks the server for updates to the mooc exercises in the given projects directory.
///
/// A course courses.mooc.fi refuses lands in `unchecked_courses` and the others are still
/// checked; any other failure fails the whole check.
pub fn check_mooc_exercise_updates(
    client: &mooc::MoocClient,
    auth: &MoocAuth,
    projects_dir: &Path,
) -> Result<MoocExerciseUpdates, LangsError> {
    log::debug!("checking exercise updates in {}", projects_dir.display());

    let mut updates = MoocExerciseUpdates::default();

    let config = ProjectsConfig::load(projects_dir)?;

    for course_config in config.mooc_courses.values() {
        if course_config.exercises.is_empty() {
            continue;
        }
        match check_mooc_course_updates(client, auth, course_config) {
            Ok(course_updates) => {
                updates.updated.extend(course_updates.updated);
                updates.removed.extend(course_updates.removed);
            }
            Err(err) => {
                let Some(kind) = CourseRefusalKind::of(&err) else {
                    return Err(err.into());
                };
                log::warn!(
                    "skipping the update check of course {}: {err}",
                    course_config.course_id
                );
                updates.unchecked_courses.push(UncheckedMoocCourse {
                    course_id: course_config.course_id,
                    kind,
                });
            }
        }
    }
    Ok(updates)
}

/// [`check_mooc_exercise_updates`] for one course; `unchecked_courses` stays empty.
fn check_mooc_course_updates(
    client: &mooc::MoocClient,
    auth: &MoocAuth,
    course_config: &MoocCourseConfig,
) -> Result<MoocExerciseUpdates, MoocAuthFailure> {
    let mut updates = MoocExerciseUpdates::default();
    // Keyed by exercise id: the stored `task_id` is the editor task's id, not an exercise key.
    let server_exercises: HashMap<Uuid, mooc::TmcExerciseSlide> =
        stored_course_exercises(client, auth, course_config)?
            .into_iter()
            .map(|slide| (slide.exercise_id, slide))
            .collect();
    let mut missing = Vec::new();
    for (exercise_id, local_exercise) in &course_config.exercises {
        let Some(server_exercise) = server_exercises.get(exercise_id) else {
            missing.push(*exercise_id);
            continue;
        };
        if server_exercise
            .editor_checksum()
            .map(|cs| cs != local_exercise.checksum)
            .unwrap_or_default()
        {
            updates.updated.push(*exercise_id);
        }
    }
    if !missing.is_empty() && !course_config.is_exam {
        let closed = closed_chapter_exercise_ids(client, auth, course_config.course_id)?;
        updates
            .removed
            .extend(missing.into_iter().filter(|id| !closed.contains(id)));
    }
    Ok(updates)
}

/// The current exercise list of a course or exam stored in the projects directory.
fn stored_course_exercises(
    client: &MoocClient,
    auth: &MoocAuth,
    course_config: &MoocCourseConfig,
) -> Result<Vec<mooc::TmcExerciseSlide>, MoocAuthFailure> {
    if course_config.is_exam {
        auth.call(client, |c| c.exam_exercises(course_config.course_id))
    } else {
        auth.call(client, |c| c.course_exercises(course_config.course_id))
    }
}

/// The course's exercises that sit in chapters that have not opened yet. The course's exercise
/// list omits them, so they must not be mistaken for removed exercises.
fn closed_chapter_exercise_ids(
    client: &MoocClient,
    auth: &MoocAuth,
    course_id: Uuid,
) -> Result<HashSet<Uuid>, MoocAuthFailure> {
    let progress = auth.call(client, |c| c.course_progress(course_id))?;
    Ok(progress
        .closed_chapters
        .iter()
        .flat_map(|chapter| chapter.exercises.iter().map(|e| e.exercise_id))
        .collect())
}

/// Downloads the user's old submission from the server and restores it over a fresh template at
/// the path, replacing the directory's contents.
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

    // Fetch both archives before touching the exercise: a failed download must not wipe the
    // student's work.
    let mut submission = vec![];
    client.download_old_submission(submission_id, &mut submission)?;
    let mut template = vec![];
    client.download_exercise(exercise_id, &mut template)?;
    log::debug!("downloaded old submission");

    let staging = staging_dir_beside(output_path)?;
    extract_project(
        Cursor::new(template),
        staging.path(),
        Compression::Zip,
        false,
        false,
    )?;
    extract_student_files(Cursor::new(submission), Compression::Zip, staging.path())?;
    replace_dir_contents(output_path, staging.path())?;
    log::debug!("restored old submission");
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
        ProjectsConfig::resolve_tmc_exercise_path(projects_dir, course_slug, exercise_slug);

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
        ProjectsConfig::resolve_tmc_exercise_path(projects_dir, course_slug, exercise_slug);

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
    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
    let projects_config = ProjectsConfig::load(projects_dir)?;

    // separate exercises into downloads and skipped
    let mut to_be_downloaded = vec![];
    let mut to_be_skipped = vec![];

    log::debug!("checking the checksum of each exercise on the server");
    for exercise_detail in exercises_details {
        let target = ProjectsConfig::resolve_tmc_exercise_path(
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

    let course_ids = Arc::new(lookup_unrecorded_tmc_course_ids(
        client,
        &projects_config,
        &to_be_downloaded,
    ));

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
        let course_ids = Arc::clone(&course_ids);
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
                            extract_update(buf, &download_target.target.path, Compression::Zip)?;
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
                    if let Some(course_id) = course_ids.get(&download_target.target.course_slug) {
                        course_config.course_id = Some(*course_id);
                    }
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
        let failed = failed
            .into_iter()
            .map(|(target, err)| {
                let failure = TmcDownloadFailure::of(&err);
                (target.target, error_chain(&err), failure)
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
/// The tmc root URL comes from the environment and so does not prove the host is
/// tmc-server; this list stops a redirected URL from harvesting the token.
const TMC_HOSTS_TRUSTED_WITH_MOOC_TOKEN: &[&str] = &["tmc.mooc.fi"];

/// See [`TMC_HOSTS_TRUSTED_WITH_MOOC_TOKEN`]. Loopback needs the same opt-in as
/// the mooc client, so production never trusts a local host implicitly.
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
    /// For the 401 handling, which may delete a rejected *tmc* token but must never
    /// touch the mooc credentials: tmc-server rejecting a mooc token says nothing
    /// about its validity at courses.mooc.fi.
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
/// A stored tmc `credentials.json` wins while it exists, so password-era logins
/// keep working until tmc-server rejects the token; the 401 handling then deletes
/// the file and the next invocation falls through to the mooc token. The
/// credential never changes under a session that still works.
///
/// `mooc_root_url` and `mooc_client_id` are used to refresh the mooc token, as in
/// [`init_mooc_client_with_credentials`].
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
/// The stored token is refreshed first if expired (see
/// [`MoocCredentials::load_valid`]). `client_id` is the OAuth2 client id the
/// refresh grant is made with.
pub fn init_mooc_client_with_credentials(
    root_url: Url,
    client_name: &str,
    client_id: &str,
) -> Result<(mooc::MoocClient, Option<MoocCredentials>), LangsError> {
    // create client
    let mut client = mooc::MoocClient::new(root_url.clone())?;

    let credentials = MoocCredentials::load_valid(client_name, &root_url, client_id)?;
    if let Some(credentials) = &credentials {
        client.set_token(credentials.token());
    }

    Ok((client, credentials))
}

/// Looks up the TMC id of each course in `targets` whose config records none yet, keyed by course
/// slug. Costs one request per such course; a failed lookup only leaves that id unrecorded.
fn lookup_unrecorded_tmc_course_ids(
    client: &tmc::TestMyCodeClient,
    projects_config: &ProjectsConfig,
    targets: &[DownloadTarget],
) -> HashMap<String, u32> {
    let mut looked_up = HashMap::<String, Option<u32>>::new();
    for target in targets {
        let course_slug = &target.target.course_slug;
        let is_recorded = projects_config
            .tmc_courses
            .get(course_slug)
            .is_some_and(|course| course.course_id.is_some());
        if is_recorded || looked_up.contains_key(course_slug) {
            continue;
        }
        let course_id = match client.get_exercise_details(target.target.id) {
            Ok(details) => Some(details.course_id),
            Err(err) => {
                log::warn!("Failed to look up the course id of {course_slug}: {err}");
                None
            }
        };
        looked_up.insert(course_slug.clone(), course_id);
    }
    looked_up
        .into_iter()
        .filter_map(|(course_slug, course_id)| Some((course_slug, course_id?)))
        .collect()
}

/// Updates the tmc exercises in the local projects directory.
// TODO: parallel downloads
pub fn update_tmc_exercises(
    client: &tmc::TestMyCodeClient,
    projects_dir: &Path,
) -> Result<DownloadOrUpdateTmcCourseExercisesResult, LangsError> {
    log::debug!("updating exercises in {}", projects_dir.display());

    let mut course_data = HashMap::<String, Vec<(String, String, u32)>>::new();

    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
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
                let Some(server_exercise) = tmc_server_exercises.remove(&local_exercise.id) else {
                    log::info!(
                        "skipping exercise {}, the server no longer has it",
                        local_exercise.id
                    );
                    continue;
                };
                if server_exercise.checksum != local_exercise.checksum {
                    // server has an updated exercise
                    let target = ProjectsConfig::resolve_tmc_exercise_path(
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
                extract_update(buf, &exercise.path, Compression::Zip)?;
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
                        course_id: None,
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

/// One exercise [`download_or_update_mooc_course_exercises`] downloads.
struct PlannedMoocDownload {
    exercise_id: Uuid,
    course_id: Uuid,
    exercise_name: String,
    task_id: Uuid,
    checksum: String,
    exercise_directory: String,
    download_url: String,
    target: PathBuf,
}

/// Mirrors the TMC path's download thread count.
const MOOC_DOWNLOAD_THREADS: usize = 4;

/// Downloads and extracts each planned exercise, a few at a time; results are in plan order.
fn download_mooc_archives_in_parallel(
    client: &MoocClient,
    auth: &MoocAuth,
    planned: &[PlannedMoocDownload],
) -> Vec<Result<(), DownloadArchiveError>> {
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, Result<(), DownloadArchiveError>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..MOOC_DOWNLOAD_THREADS.min(planned.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut results = vec![];
                    loop {
                        let index = next.fetch_add(1, AtomicOrdering::Relaxed);
                        let Some(plan) = planned.get(index) else {
                            break;
                        };
                        progress_reporter::progress_stage::<mooc::MoocClientUpdateData>(
                            format!(
                                "Downloading exercise {} to '{}'",
                                plan.exercise_id,
                                plan.target.display()
                            ),
                            Some(mooc::MoocClientUpdateData::ExerciseDownload {
                                id: plan.exercise_id,
                                path: plan.target.clone(),
                            }),
                        );
                        let result = download_and_extract_mooc_archive(
                            client,
                            auth,
                            &plan.download_url,
                            &plan.target,
                        );
                        results.push((index, result));
                    }
                    results
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("download workers do not panic"))
            .collect()
    });
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// Where [`download_or_update_mooc_course_exercises`] looks the requested exercises up.
#[derive(Debug, Clone, Copy)]
pub enum MoocExerciseSource {
    Course(Uuid),
    /// An exam from [`mooc::MoocClient::exams`]; its exercises are stored like a course's.
    Exam(Uuid),
    /// Every course the user is enrolled on, in turn.
    EnrolledCourses,
}

/// The course or exam an exercise is stored under.
#[derive(Debug, Clone)]
struct StoredCourse {
    id: Uuid,
    name: String,
    is_exam: bool,
}

impl StoredCourse {
    fn of_course(course: &mooc::Course) -> Self {
        Self {
            id: course.id,
            name: course.name.clone(),
            is_exam: false,
        }
    }
}

/// Updates the mooc exercises in the local projects directory.
pub fn update_mooc_exercises(
    client: &MoocClient,
    auth: &MoocAuth,
    projects_dir: &Path,
) -> Result<DownloadOrUpdateMoocCourseExercisesResult, LangsError> {
    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
    let mut projects_config = ProjectsConfig::load(projects_dir)?;
    // Owned snapshot keyed by exercise id (not the editor task id), so
    // `projects_config` can be mutated below.
    struct LocalMoocExerciseInfo {
        course_id: Uuid,
        course_directory: String,
        exercise_directory: String,
        exercise_name: String,
        checksum: String,
    }
    let locals: HashMap<Uuid, LocalMoocExerciseInfo> = projects_config
        .mooc_courses
        .iter()
        .flat_map(|(course_id, cc)| {
            cc.exercises.iter().map(move |(id, e)| {
                (
                    *id,
                    LocalMoocExerciseInfo {
                        course_id: *course_id,
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
        // One exercise list request per course, not one per exercise.
        let mut server_exercises: Vec<mooc::TmcExerciseSlide> = Vec::new();
        for course_config in projects_config.mooc_courses.values() {
            match stored_course_exercises(client, auth, course_config) {
                Ok(slides) => server_exercises.extend(slides),
                // An exam stops being served once it ends; that must not block the other courses.
                Err(err) if CourseRefusalKind::of(&err).is_some() => {
                    log::warn!(
                        "skipping the update of course {}: {err}",
                        course_config.course_id
                    );
                }
                Err(err) => return Err(err.into()),
            }
        }
        for slide in server_exercises {
            let Some(local) = locals.get(&slide.exercise_id) else {
                // Not tracked locally (e.g. deleted).
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
            // The slide already carries the stub URL; no extra fetch.
            let Some(download_url) = slide.editor_stub_download_url() else {
                log::warn!(
                    "Skipping exercise {}: no downloadable editor task in public spec",
                    slide.exercise_id
                );
                continue;
            };
            download_and_extract_mooc_archive(client, auth, download_url, &target)?;

            // Persist the new checksum, or every later check re-reports the update.
            // Prefer the editor task id (the checksum's owner) over the stored one.
            let task_id = slide
                .editor_task_id()
                .or_else(|| {
                    projects_config
                        .mooc_courses
                        .get(&local.course_id)
                        .and_then(|cc| cc.exercises.get(&slide.exercise_id))
                        .map(|e| e.task_id)
                })
                .unwrap_or_else(Uuid::nil);
            if let Some(course_config) = projects_config.mooc_courses.get_mut(&local.course_id) {
                course_config.add_exercise(
                    slide.exercise_id,
                    local.exercise_name.clone(),
                    task_id,
                    new_checksum.to_string(),
                    local.exercise_directory.clone(),
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
    })
}

impl TmcDownloadFailure {
    fn of(err: &LangsError) -> Self {
        match err {
            LangsError::TestMyCodeClient(err) => match err.as_ref() {
                tmc::TestMyCodeClientError::HttpError { status, .. }
                    if matches!(status.as_u16(), 403 | 404) =>
                {
                    Self::NotFound
                }
                tmc::TestMyCodeClientError::HttpError { status, .. }
                    if status.is_server_error() =>
                {
                    Self::ServerError
                }
                tmc::TestMyCodeClientError::ConnectionError(..) => Self::ConnectionError,
                _ => Self::Other,
            },
            LangsError::Tmc(_)
            | LangsError::FileError(_)
            | LangsError::Plugin(_)
            | LangsError::Zip(_) => Self::ExtractFailed,
            _ => Self::Other,
        }
    }
}

/// The [`MoocDownloadFailure`] of an error downloading or extracting a mooc exercise archive.
fn mooc_download_failure(err: &LangsError) -> MoocDownloadFailure {
    match err {
        LangsError::MoocClient(err) => match err.as_ref() {
            mooc::MoocClientError::FileNotFound { .. } => MoocDownloadFailure::FileMissing,
            mooc::MoocClientError::ConnectionError(..) => MoocDownloadFailure::ConnectionError,
            mooc::MoocClientError::HttpError { status, .. } if status.is_server_error() => {
                MoocDownloadFailure::ServerError
            }
            _ => MoocDownloadFailure::Other,
        },
        LangsError::Tmc(_)
        | LangsError::FileError(_)
        | LangsError::Plugin(_)
        | LangsError::Zip(_) => MoocDownloadFailure::ExtractFailed,
        _ => MoocDownloadFailure::Other,
    }
}

/// Flattens an error's `source()` chain into strings, outermost first, for the
/// CLI's `failed` list entries.
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

/// Downloads the `.tar.zst` stub archive at `download_url` (an editor task's
/// `stub_download_url`) and extracts it into `target`, cleaning out what an
/// earlier version of the exercise there left behind.
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
    extract_update(data, target, Compression::TarZstd).map_err(DownloadArchiveError::Local)?;
    Ok(())
}

/// Restores a past mooc submission at `output_path`: the submission's student
/// files overlaid on a fresh exercise stub, fully replacing the directory's contents.
///
/// Mirrors [`download_old_submission`] minus the server reset (mooc has none). If
/// `save_old_state` is set, the current state is first stored as an ungraded backup
/// snapshot so nothing the student wrote is lost.
///
/// A submission with no files yields [`MoocOldSubmissionRestore::NothingToDownload`];
/// one with several yields [`LangsError::NotATmcAnswer`]. Both are decided before
/// the local exercise or the server is touched.
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

    // A tmc answer is exactly one `TarZstd` archive; guessing among several files
    // would silently restore the wrong project.
    let mut files = auth.call(client, |c| c.download_submission_files(submission_id))?;
    let archive_url = match files.len() {
        0 => {
            log::debug!("submission {submission_id} has no downloadable files");
            return Ok(MoocOldSubmissionRestore::NothingToDownload);
        }
        1 => files.remove(0).url,
        file_count => {
            return Err(LangsError::NotATmcAnswer {
                submission_id,
                file_count,
            });
        }
    };

    if save_old_state {
        let temp = file_util::named_temp_file()?;
        compress_project_to(output_path, temp.path(), Compression::TarZstd, false, false)?;
        auth.call(client, |c| {
            c.store_snapshot(exercise_id, temp.path(), mooc::api::SnapshotPurpose::Backup)
        })?;
        log::debug!("backed up current state before downloading old submission");
    }

    let staging = staging_dir_beside(output_path)?;
    let base = staging.path();
    let stub = auth.call(client, |c| c.download_exercise(exercise_id))?;
    extract_project(Cursor::new(stub), base, Compression::TarZstd, false, false)?;
    log::debug!("extracted fresh stub to temp base");

    let url = Url::parse(&archive_url)
        .map_err(|err| Box::new(mooc::MoocClientError::UrlParse(archive_url.clone(), err)))?;
    let archive = auth.call(client, |c| c.download(url.clone()))?;
    extract_student_files(Cursor::new(archive), Compression::TarZstd, base)?;
    log::debug!("overlaid old submission student files");

    replace_dir_contents(output_path, base)?;
    log::debug!("moved restored exercise into place");
    Ok(MoocOldSubmissionRestore::Restored)
}

/// Downloads or updates the given mooc exercises in the local projects directory.
///
/// With `course_id` (which the extension always knows) only that course's slides
/// are fetched; otherwise the enrolled courses are scanned (O(courses x
/// exercises)), since the langs API has no exercise -> course lookup. The course
/// name determines the on-disk directory, and the course id doubles as the
/// projects config instance key (the API does not expose course instance ids).
///
/// Per exercise: not found, in a chapter that has not opened yet, or no editor task (e.g.
/// browser-only) -> failed;
/// stored checksum matches -> skipped; otherwise the stub archive is downloaded,
/// extracted and the course config updated. Results are keyed by the requested
/// `exercise_id`.
pub fn download_or_update_mooc_course_exercises(
    client: &MoocClient,
    auth: &MoocAuth,
    projects_dir: &Path,
    exercise_ids: &[Uuid],
    source: MoocExerciseSource,
) -> Result<DownloadOrUpdateMoocCourseExercisesResult, LangsError> {
    log::debug!(
        "downloading or updating {} mooc exercises in {}",
        exercise_ids.len(),
        projects_dir.display()
    );

    let mut config_update_lock = ProjectsConfig::lock_course_config_updates(projects_dir)?;
    let _config_update_guard = config_update_lock.lock()?;
    let mut projects_config = ProjectsConfig::load(projects_dir)?;

    let requested: HashSet<Uuid> = exercise_ids.iter().copied().collect();
    let mut resolved: HashMap<Uuid, (StoredCourse, mooc::TmcExerciseSlide)> = HashMap::new();
    // Requested exercises of a chapter that has not opened yet, with why they cannot be downloaded.
    let mut closed: HashMap<Uuid, String> = HashMap::new();
    if !requested.is_empty() {
        match source {
            MoocExerciseSource::Course(course_id) => {
                let course = StoredCourse::of_course(&auth.call(client, |c| c.course(course_id))?);
                for slide in auth.call(client, |c| c.course_exercises(course_id))? {
                    if requested.contains(&slide.exercise_id) {
                        resolved.insert(slide.exercise_id, (course.clone(), slide));
                    }
                }
                if resolved.len() < requested.len() {
                    let progress = auth.call(client, |c| c.course_progress(course_id))?;
                    for chapter in progress.closed_chapters {
                        for exercise in &chapter.exercises {
                            if requested.contains(&exercise.exercise_id) {
                                closed.insert(
                                    exercise.exercise_id,
                                    format!(
                                        "Exercise {} is in chapter {} \"{}\", which opens at {}",
                                        exercise.exercise_id,
                                        chapter.chapter_number,
                                        chapter.name,
                                        chapter.opens_at
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            MoocExerciseSource::Exam(exam_id) => {
                let exam = auth
                    .call(client, |c| c.exams())?
                    .into_iter()
                    .find(|exam| exam.id == exam_id)
                    .ok_or(LangsError::MoocExamNotAnswerable(exam_id))?;
                let exam = StoredCourse {
                    id: exam.id,
                    name: exam.name,
                    is_exam: true,
                };
                for slide in auth.call(client, |c| c.exam_exercises(exam_id))? {
                    if requested.contains(&slide.exercise_id) {
                        resolved.insert(slide.exercise_id, (exam.clone(), slide));
                    }
                }
            }
            MoocExerciseSource::EnrolledCourses => {
                'courses: for course in auth.call(client, |c| c.courses())? {
                    let stored_course = StoredCourse::of_course(&course);
                    for slide in auth.call(client, |c| c.course_exercises(course.id))? {
                        if requested.contains(&slide.exercise_id)
                            && !resolved.contains_key(&slide.exercise_id)
                        {
                            resolved.insert(slide.exercise_id, (stored_course.clone(), slide));
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
    let mut failed: Vec<(MoocExerciseDownload, Vec<String>, MoocDownloadFailure)> = Vec::new();

    let total_steps = u32::try_from(exercise_ids.len())
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    progress_reporter::start_stage::<mooc::MoocClientUpdateData>(
        total_steps,
        format!("Downloading {} mooc exercises", exercise_ids.len()),
        None,
    );

    let mut planned = Vec::new();
    // Directories given to exercises earlier in this batch, which are not in the config yet.
    let mut reserved_directories: HashMap<Uuid, HashSet<String>> = HashMap::new();
    for &exercise_id in exercise_ids {
        let Some((course, slide)) = resolved.get(&exercise_id) else {
            let (reason, failure) = match closed.remove(&exercise_id) {
                Some(reason) => (reason, MoocDownloadFailure::ClosedChapter),
                None => (
                    format!(
                        "Exercise {exercise_id} was not found in any of the user's enrolled \
                         courses"
                    ),
                    MoocDownloadFailure::NotFound,
                ),
            };
            failed.push((
                MoocExerciseDownload {
                    exercise_id,
                    path: projects_dir.join("mooc"),
                },
                vec![reason],
                failure,
            ));
            continue;
        };

        let (task_id, download_url) = match slide.editor_download() {
            Ok(editor_download) => editor_download,
            Err(failure) => {
                let reason = match failure {
                    MoocDownloadFailure::SeveralEditorTasks => {
                        mooc::MoocClientError::SeveralEditorTasks { exercise_id }.to_string()
                    }
                    _ => format!(
                        "Exercise {exercise_id} has no downloadable editor task \
                         (browser exercises have no project archive)"
                    ),
                };
                failed.push((
                    MoocExerciseDownload {
                        exercise_id,
                        path: projects_dir.join("mooc"),
                    },
                    vec![reason],
                    failure,
                ));
                continue;
            }
        };
        let download_url = download_url.to_string();
        let checksum = slide.editor_checksum().unwrap_or_default().to_string();
        let exercise_name = slide.exercise_name.clone();

        let course_config = projects_config.get_or_init_mooc_course_config(
            course.id,
            course.name.clone(),
            course.is_exam,
        );

        let reserved = reserved_directories.entry(course.id).or_default();
        let exercise_directory =
            course_config.exercise_directory(exercise_id, &exercise_name, reserved.iter());
        let target = ProjectsConfig::get_mooc_exercise_download_target(
            projects_dir,
            &course_config.directory,
            &exercise_directory,
        );

        if let Some(existing) = course_config.exercises.get(&exercise_id) {
            if existing.checksum == checksum && existing.directory == exercise_directory {
                log::info!("Skipping exercise {exercise_id} due to identical checksum");
                skipped.push(MoocExerciseDownload {
                    exercise_id,
                    path: target,
                });
                continue;
            }
        }

        reserved.insert(exercise_directory.clone());
        planned.push(PlannedMoocDownload {
            exercise_id,
            course_id: course.id,
            exercise_name,
            task_id,
            checksum,
            exercise_directory,
            download_url,
            target,
        });
    }

    let results = download_mooc_archives_in_parallel(client, auth, &planned);
    let mut permanent_auth_failure = None;
    let mut changed_courses = HashSet::new();
    for (plan, result) in planned.into_iter().zip(results) {
        let exercise_id = plan.exercise_id;
        match result {
            Ok(()) => {
                if let Some(course_config) = projects_config.mooc_courses.get_mut(&plan.course_id) {
                    course_config.add_exercise(
                        exercise_id,
                        plan.exercise_name,
                        plan.task_id,
                        plan.checksum,
                        plan.exercise_directory,
                    );
                    changed_courses.insert(plan.course_id);
                }
                downloaded.push(MoocExerciseDownload {
                    exercise_id,
                    path: plan.target,
                });
            }
            // No session left, so the other downloads failed the same way: surface the auth
            // failure instead of a partial result. Finished downloads are still recorded below
            // and skipped on retry.
            Err(DownloadArchiveError::Auth(auth_err)) if auth_err.is_permanent() => {
                permanent_auth_failure.get_or_insert(auth_err);
            }
            Err(err) => {
                let err: LangsError = err.into();
                let failure = mooc_download_failure(&err);
                failed.push((
                    MoocExerciseDownload {
                        exercise_id,
                        path: plan.target,
                    },
                    error_chain(&err),
                    failure,
                ));
            }
        }
    }
    for course_id in changed_courses {
        if let Some(course_config) = projects_config.mooc_courses.get(&course_id) {
            course_config.save_to_projects_dir(projects_dir)?;
        }
    }
    if let Some(auth_err) = permanent_auth_failure {
        progress_reporter::finish_stage::<mooc::MoocClientUpdateData>(
            "Stopped downloading mooc exercises: not logged in".to_string(),
            None,
        );
        log::error!(
            "mooc auth permanently failed while downloading exercises ({} of {} downloaded, {} \
             skipped): {auth_err}",
            downloaded.len(),
            exercise_ids.len(),
            skipped.len(),
        );
        return Err(auth_err.into());
    }

    progress_reporter::finish_stage::<mooc::MoocClientUpdateData>(
        format!("Finished downloading {} mooc exercises", exercise_ids.len()),
        None,
    );

    Ok(DownloadOrUpdateMoocCourseExercisesResult {
        downloaded,
        skipped,
        failed: if failed.is_empty() {
            None
        } else {
            Some(failed)
        },
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

/// Resets the given exercise to a fresh template, keeping the exercise directory itself (see
/// [`replace_dir_contents`]). The directory is untouched if the download fails.
pub fn reset(
    client: &tmc::TestMyCodeClient,
    exercise_id: u32,
    exercise_path: &Path,
) -> Result<(), LangsError> {
    let mut buf = vec![];
    client.download_exercise(exercise_id, &mut buf)?;
    let staging = staging_dir_beside(exercise_path)?;
    extract_project(
        Cursor::new(buf),
        staging.path(),
        Compression::Zip,
        false,
        false,
    )?;
    replace_dir_contents(exercise_path, staging.path())
}

/// Updates the exercise at `target` to the archive's version, keeping student files (see
/// [`extract_project`] with `clean`). Also downloads a new exercise.
///
/// Works on a copy of `target` that is swapped in whole, so a held file or a killed process
/// never leaves the exercise half-updated or with truncated files.
fn extract_update(
    archive: impl AsRef<[u8]>,
    target: &Path,
    compression: Compression,
) -> Result<(), LangsError> {
    let staging = staging_dir_beside(target)?;
    if target.is_dir() {
        file_util::copy_dir_contents(target, staging.path(), || {})?;
    }
    extract_project(
        Cursor::new(archive),
        staging.path(),
        compression,
        true,
        false,
    )?;
    replace_dir_contents(target, staging.path())
}

/// How old a leftover `.tmc-staging-*` directory must be before it is treated as abandoned
/// rather than as another running command's.
const STALE_STAGING_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Deletes `.tmc-staging-*` directories in `dir` that a killed command left behind.
fn sweep_stale_staging_dirs(dir: &Path, min_age: std::time::Duration) {
    let Ok(entries) = file_util::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_DIR_PREFIX)
        {
            continue;
        }
        let is_stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= min_age);
        if is_stale && entry.file_type().is_ok_and(|t| t.is_dir()) {
            log::warn!(
                "deleting abandoned staging directory {}",
                entry.path().display()
            );
            if let Err(err) = file_util::remove_dir_all(entry.path()) {
                log::warn!("failed to delete it: {err}");
            }
        }
    }
}

const STAGING_DIR_PREFIX: &str = ".tmc-staging-";
const REPLACED_DIR_PREFIX: &str = ".tmc-replaced-";

/// The existing parent of `path`, where its staging directories go.
fn staging_parent(path: &Path) -> Result<&Path, LangsError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    file_util::create_dir_all(parent)?;
    Ok(parent)
}

/// A temporary directory beside `path`, on its filesystem, for content to move into `path`
/// by rename. Sweeps abandoned ones from earlier runs first.
fn staging_dir_beside(path: &Path) -> Result<tempfile::TempDir, LangsError> {
    let parent = staging_parent(path)?;
    sweep_stale_staging_dirs(parent, STALE_STAGING_AGE);
    let staging = tempfile::Builder::new()
        .prefix(STAGING_DIR_PREFIX)
        .tempdir_in(parent)
        .map_err(FileError::TempFile)?;
    Ok(staging)
}

/// Replaces the entries of `target` with those of `staged` (a [`staging_dir_beside`]
/// `target`), keeping `target` itself: an editor watching the exercise directory loses
/// track of it when the directory is replaced.
///
/// Leaves `target` as it was on failure. If even that fails, the old entries are kept in a
/// `.tmc-replaced-*` directory beside `target` and the error names it.
fn replace_dir_contents(target: &Path, staged: &Path) -> Result<(), LangsError> {
    if !target.exists() {
        file_util::rename(staged, target)?;
        return Ok(());
    }
    // Not a guard and never swept: it holds the student's only copy of the old files until
    // they are moved back or the swap completes.
    let replaced = tempfile::Builder::new()
        .prefix(REPLACED_DIR_PREFIX)
        .tempdir_in(staging_parent(target)?)
        .map_err(FileError::TempFile)?
        .keep();

    if let Err((moved_out, err)) = move_dir_entries(target, &replaced) {
        restore_dir_entries(&replaced, target, &moved_out, &err)?;
        return Err(err.into());
    }
    if let Err((moved_in, err)) = move_dir_entries(staged, target) {
        for name in &moved_in {
            let _ = file_util::remove_all(target.join(name));
        }
        let moved_out = dir_entry_names(&replaced)?;
        restore_dir_entries(&replaced, target, &moved_out, &err)?;
        return Err(err.into());
    }
    if let Err(err) = file_util::remove_dir_all(&replaced) {
        log::warn!(
            "replaced {} but could not delete the old files in {}: {err}",
            target.display(),
            replaced.display()
        );
    }
    Ok(())
}

/// Moves `names` from `replaced` back into `target` after a failed [`replace_dir_contents`].
fn restore_dir_entries(
    replaced: &Path,
    target: &Path,
    names: &[OsString],
    cause: &FileError,
) -> Result<(), LangsError> {
    let mut restore_failed = false;
    for name in names {
        if let Err(err) = file_util::rename(replaced.join(name), target.join(name)) {
            log::error!("failed to restore {}: {err}", target.join(name).display());
            restore_failed = true;
        }
    }
    if restore_failed {
        return Err(LangsError::ExerciseRestoreFailed {
            exercise_path: target.to_path_buf(),
            kept_at: replaced.to_path_buf(),
            cause: cause.to_string(),
        });
    }
    let _ = file_util::remove_dir_empty(replaced);
    Ok(())
}

fn dir_entry_names(dir: &Path) -> Result<Vec<OsString>, FileError> {
    file_util::read_dir(dir)?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|err| FileError::DirRead(dir.to_path_buf(), err))
        })
        .collect()
}

/// Moves every entry of `from` into `to` by rename. On failure, returns the names already
/// moved with the error.
fn move_dir_entries(from: &Path, to: &Path) -> Result<Vec<OsString>, (Vec<OsString>, FileError)> {
    let names = dir_entry_names(from).map_err(|err| (vec![], err))?;
    let mut moved = Vec::with_capacity(names.len());
    for name in names {
        if let Err(err) = file_util::rename(from.join(&name), to.join(&name)) {
            return Err((moved, err));
        }
        moved.push(name);
    }
    Ok(moved)
}

/// Moves every entry of `from` into `to` by rename, moving the ones already moved back if one
/// cannot be.
pub(crate) fn move_all_dir_entries(from: &Path, to: &Path) -> Result<(), LangsError> {
    let Err((moved, err)) = move_dir_entries(from, to) else {
        return Ok(());
    };
    let mut is_split = false;
    for name in moved {
        if let Err(undo_err) = file_util::rename(to.join(&name), from.join(&name)) {
            log::error!(
                "failed to move {} back: {undo_err}",
                to.join(&name).display()
            );
            is_split = true;
        }
    }
    if is_split {
        return Err(LangsError::DirEntriesSplit {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            cause: err.to_string(),
        });
    }
    Err(err.into())
}

/// Resets a mooc exercise to a fresh stub, optionally storing the current state
/// as an ungraded backup snapshot first. Mirrors [`reset`].
///
/// Never leaves `exercise_path` half-written: the stub is extracted into a staging sibling
/// before the original is touched, then swapped in by [`replace_dir_contents`].
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
        let temp = file_util::named_temp_file()?;
        compress_project_to(
            exercise_path,
            temp.path(),
            Compression::TarZstd,
            false,
            false,
        )?;
        auth.call(client, |c| {
            c.store_snapshot(exercise_id, temp.path(), mooc::api::SnapshotPurpose::Backup)
        })?;
        log::debug!("backed up current state before resetting exercise");
    }

    // Fetch before touching the directory: a download failure must not wipe
    // existing work.
    let stub = auth.call(client, |c| c.download_exercise(exercise_id))?;
    let staging = staging_dir_beside(exercise_path)?;
    extract_project(
        Cursor::new(stub),
        staging.path(),
        Compression::TarZstd,
        false,
        false,
    )?;
    replace_dir_contents(exercise_path, staging.path())
}

/// Extracts the compressed project to the target location.
///
/// With `clean`, files at the target that the archive lacks are removed unless they are
/// student files; see [`LanguagePlugin::extract_project`] for exactly what survives.
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
        // A checkout in a temporary directory has a hidden name.
        (e.depth() == 0 || !submission_processing::is_hidden_dir(e))
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

/// Moves the directory `source` to `target`, which may exist if empty. `record_move` runs once
/// the contents are at `target`, before the old directory is deleted, so a failure never leaves
/// the caller's records pointing at a deleted directory.
fn move_dir(
    source: &Path,
    target: &Path,
    record_move: impl FnOnce() -> Result<(), LangsError>,
) -> Result<(), LangsError> {
    move_dir_with(source, target, true, record_move)
}

/// [`move_dir`], where `try_rename: false` forces the copy fallback used across volumes.
fn move_dir_with(
    source: &Path,
    target: &Path,
    try_rename: bool,
    record_move: impl FnOnce() -> Result<(), LangsError>,
) -> Result<(), LangsError> {
    if try_rename {
        let is_empty_target = target.is_dir() && dir_entry_names(target)?.is_empty();
        if is_empty_target {
            file_util::remove_dir_empty(target)?;
        }
        if let Some(parent) = target.parent() {
            file_util::create_dir_all(parent)?;
        }
        match file_util::rename(source, target) {
            Ok(()) => {
                if let Err(err) = record_move() {
                    if let Err(undo_err) = file_util::rename(target, source) {
                        log::error!("failed to move {} back: {undo_err}", target.display());
                    }
                    return Err(err);
                }
                return Ok(());
            }
            Err(err) if file_util::is_cross_volume_error(&err) => {
                log::debug!("moving across volumes, copying instead: {err}");
            }
            Err(err) => {
                if is_empty_target {
                    file_util::create_dir_all(target)?;
                }
                return Err(err.into());
            }
        }
    }

    file_util::create_dir_all(target)?;
    let result = copy_contents_for_move(source, target).and_then(|()| record_move());
    if let Err(err) = result {
        for name in dir_entry_names(target).unwrap_or_default() {
            if let Err(clean_err) = file_util::remove_all(target.join(name)) {
                log::warn!("failed to clean up the partial copy: {clean_err}");
            }
        }
        return Err(err);
    }

    for name in dir_entry_names(source).unwrap_or_default() {
        if let Err(err) = file_util::remove_all(source.join(&name)) {
            log::warn!(
                "failed to delete {} after the move: {err}",
                source.display()
            );
        }
    }
    if let Err(err) = file_util::remove_dir_empty(source) {
        log::warn!(
            "failed to delete {} after the move: {err}",
            source.display()
        );
    }
    finish_stage("Finished moving project directory");
    Ok(())
}

fn copy_contents_for_move(source: &Path, target: &Path) -> Result<(), LangsError> {
    let mut file_count_total = 0;
    for entry in WalkDir::new(source) {
        if !entry?.file_type().is_dir() {
            file_count_total += 1;
        }
    }
    start_stage(
        file_count_total + 1,
        format!("Moving dir {} -> {}", source.display(), target.display()),
    );

    let mut file_count_copied = 0;
    file_util::copy_dir_contents(source, target, || {
        file_count_copied += 1;
        progress_stage(format!(
            "Moved file {file_count_copied} / {file_count_total}"
        ));
    })?;
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
    Archive::new(compressed_project, compression)?.extract(target_location)?;
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

    /// A `MoocAuth` pointed at the mock server; the client id and name are
    /// arbitrary since no test here exercises a 401/refresh.
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

    /// Writes a mooc course config with one exercise and creates its directory so
    /// the loader doesn't prune it. Returns (course_id, exercise_id, task_id).
    fn write_mooc_course_config(
        projects_dir: &std::path::Path,
        local_checksum: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let course_id = Uuid::new_v4();
        let exercise_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        file_to(
            projects_dir,
            "mooc/my-course/course_config.toml",
            format!(
                r#"
course_id = "{course_id}"
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
        (course_id, exercise_id, task_id)
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

    /// JSON for a course's progress whose one closed chapter holds `closed_exercise_ids`.
    fn progress_with_closed_chapter_json(closed_exercise_ids: &[Uuid]) -> String {
        let exercises: Vec<_> = closed_exercise_ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "exercise_id": id,
                    "score_given": 0.0,
                    "score_maximum": 1,
                    "completed": false,
                    "attempted": false,
                })
            })
            .collect();
        serde_json::json!({
            "course_id": Uuid::new_v4(),
            "exercises": [],
            "closed_chapters": [{
                "id": Uuid::new_v4(),
                "name": "Later",
                "chapter_number": 2,
                "opens_at": "2030-01-01T00:00:00Z",
                "exercises": exercises,
            }],
        })
        .to_string()
    }

    /// Adds a second exercise in directory `ex-2` to the course written by
    /// [`write_mooc_course_config`], with a file in it; returns its id.
    fn add_second_mooc_exercise(projects_dir: &std::path::Path) -> Uuid {
        let exercise_id = Uuid::new_v4();
        let config_path = projects_dir.join("mooc/my-course/course_config.toml");
        let mut config = std::fs::read_to_string(&config_path).unwrap();
        config.push_str(&format!(
            r#"
[exercises."{exercise_id}"]
name = "Exercise 2"
task_id = "{}"
checksum = "old checksum"
directory = "ex-2"
"#,
            Uuid::new_v4()
        ));
        std::fs::write(&config_path, config).unwrap();
        file_to(
            projects_dir,
            "mooc/my-course/ex-2/src/main.py",
            "student code",
        );
        exercise_id
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
    fn an_exercises_folder_on_a_missing_drive_is_reported_not_recreated() {
        init();
        let config_dir = tempfile::tempdir().unwrap();
        let unplugged = config_dir.path().join("unplugged-drive/exercises");
        let _guard = crate::config::env_lock();
        // SAFETY: serialized by `config::env_lock`.
        unsafe { std::env::set_var(TMC_LANGS_CONFIG_DIR_VAR, config_dir.path()) };
        let mut config = TmcConfig::load("client").unwrap();
        config.projects_dir = unplugged.clone();
        config.save().unwrap();

        let result = get_projects_dir("client");
        // SAFETY: serialized by `config::env_lock`.
        unsafe { std::env::remove_var(TMC_LANGS_CONFIG_DIR_VAR) };
        assert!(
            matches!(result, Err(LangsError::ProjectsDirUnreachable(path)) if path == unplugged)
        );
        assert!(!unplugged.exists());
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
        assert_eq!(
            updates,
            ExerciseUpdates {
                updated: vec![1],
                removed: vec![]
            }
        );
    }

    fn tmc_details_response(details: Vec<ExercisesDetails>) -> String {
        serde_json::to_string(&HashMap::from([("exercises", details)])).unwrap()
    }

    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        for (path, contents) in files {
            zw.start_file(*path, SimpleFileOptions::default()).unwrap();
            zw.write_all(contents.as_bytes()).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    /// A course with exercise 1 ("kept", checksum 'old checksum') and exercise 2 ("removed").
    fn write_tmc_course_with_two_exercises(projects_dir: &Path) {
        file_to(
            projects_dir,
            "tmc/some course/course_config.toml",
            r#"
course = 'some course'

[exercises."kept"]
id = 1
checksum = 'old checksum'

[exercises."removed"]
id = 2
checksum = 'old checksum'
"#,
        );
        file_to(projects_dir, "tmc/some course/kept/src/main.py", "");
        file_to(projects_dir, "tmc/some course/removed/src/main.py", "");
    }

    #[test]
    fn check_tmc_exercise_updates_reports_exercises_removed_from_server() {
        init();
        let mut server = Server::new();
        let _m = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(tmc_details_response(vec![ExercisesDetails {
                id: 1,
                course_name: "some course".to_string(),
                exercise_name: "kept".to_string(),
                checksum: "new checksum".to_string(),
                hide_submission_results: false,
            }]))
            .create();
        let projects_dir = tempfile::tempdir().unwrap();
        write_tmc_course_with_two_exercises(projects_dir.path());

        let client = mock_testmycode_client(&server);
        let updates = check_tmc_exercise_updates(&client, projects_dir.path()).unwrap();

        assert_eq!(
            updates,
            ExerciseUpdates {
                updated: vec![1],
                removed: vec![2]
            }
        );
    }

    #[test]
    fn update_tmc_exercises_skips_exercises_removed_from_server() {
        init();
        let mut server = Server::new();
        let _details = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/details".to_string()),
            )
            .with_body(tmc_details_response(vec![ExercisesDetails {
                id: 1,
                course_name: "some course".to_string(),
                exercise_name: "kept".to_string(),
                checksum: "new checksum".to_string(),
                hide_submission_results: false,
            }]))
            .create();
        let _download = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/1/download".to_string()),
            )
            .with_body(make_zip(&[
                ("src/main.py", "print('new')"),
                ("test/__init__.py", ""),
            ]))
            .create();
        let projects_dir = tempfile::tempdir().unwrap();
        write_tmc_course_with_two_exercises(projects_dir.path());

        let client = mock_testmycode_client(&server);
        update_tmc_exercises(&client, projects_dir.path()).unwrap();

        let course_dir = projects_dir.path().join("tmc/some course");
        assert!(course_dir.join("kept/test/__init__.py").is_file());
        assert!(course_dir.join("removed/src/main.py").is_file());
        let config = ProjectsConfig::load(projects_dir.path()).unwrap();
        let course = &config.tmc_courses["some course"];
        assert_eq!(course.exercises["kept"].checksum, "new checksum");
        assert_eq!(course.exercises["removed"].checksum, "old checksum");
    }

    #[test]
    fn update_tmc_exercises_removes_files_the_new_package_dropped() {
        init();
        let mut server = Server::new();
        let _details = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/details".to_string()),
            )
            .with_body(tmc_details_response(vec![ExercisesDetails {
                id: 1,
                course_name: "some course".to_string(),
                exercise_name: "ex".to_string(),
                checksum: "new checksum".to_string(),
                hide_submission_results: false,
            }]))
            .create();
        let _download = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/1/download".to_string()),
            )
            .with_body(make_zip(&[
                ("src/new.py", "print('new')"),
                ("test/__init__.py", ""),
                ("test/test_new.py", "new test"),
            ]))
            .create();
        let projects_dir = tempfile::tempdir().unwrap();
        file_to(
            projects_dir.path(),
            "tmc/some course/course_config.toml",
            r#"
course = 'some course'

[exercises."ex"]
id = 1
checksum = 'old checksum'
"#,
        );
        let exercise_dir = projects_dir.path().join("tmc/some course/ex");
        file_to(&exercise_dir, "src/old.py", "student work");
        file_to(&exercise_dir, "test/__init__.py", "");
        file_to(&exercise_dir, "test/test_old.py", "old test");
        file_to(&exercise_dir, "tmc/__main__.py", "");

        let client = mock_testmycode_client(&server);
        update_tmc_exercises(&client, projects_dir.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(exercise_dir.join("src/old.py")).unwrap(),
            "student work"
        );
        assert!(exercise_dir.join("src/new.py").is_file());
        assert!(exercise_dir.join("test/test_new.py").is_file());
        assert!(!exercise_dir.join("test/test_old.py").exists());
        assert!(!exercise_dir.join("tmc").exists());
    }

    #[test]
    fn update_tmc_exercises_updates_a_legacy_layout_in_place() {
        init();
        let mut server = Server::new();
        let _details = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/details".to_string()),
            )
            .with_body(tmc_details_response(vec![ExercisesDetails {
                id: 1,
                course_name: "course1".to_string(),
                exercise_name: "ex1".to_string(),
                checksum: "new checksum".to_string(),
                hide_submission_results: false,
            }]))
            .create();
        let _download = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/1/download".to_string()),
            )
            .with_body(make_zip(&[
                ("src/main.py", "print('new')"),
                ("test/__init__.py", ""),
            ]))
            .create();
        let projects_dir = tempfile::tempdir().unwrap();
        file_to(
            projects_dir.path(),
            "course1/course_config.toml",
            "course = 'course1'\n[exercises.ex1]\nid = 1\nchecksum = 'old checksum'\n",
        );
        let exercise_dir = projects_dir.path().join("course1/ex1");
        file_to(&exercise_dir, "src/main.py", "student work");

        let client = mock_testmycode_client(&server);
        let result = update_tmc_exercises(&client, projects_dir.path()).unwrap();

        assert_eq!(result.downloaded[0].path, exercise_dir);
        assert!(exercise_dir.join("test/__init__.py").is_file());
        assert_eq!(
            std::fs::read_to_string(exercise_dir.join("src/main.py")).unwrap(),
            "student work"
        );
        assert!(!projects_dir.path().join("tmc").exists());
        let config = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert_eq!(
            config.tmc_courses["course1"].exercises["ex1"].checksum,
            "new checksum"
        );
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
    fn failed_tmc_downloads_say_why_they_failed() {
        init();
        let mut server = Server::new();
        let projects_dir = tempfile::tempdir().unwrap();
        let client = mock_testmycode_client(&server);

        let details = |id: u32| ExercisesDetails {
            id,
            checksum: "checksum".to_string(),
            course_name: "course".to_string(),
            exercise_name: format!("exercise{id}"),
            hide_submission_results: false,
        };
        let mut body = HashMap::new();
        body.insert("exercises", vec![details(1), details(2), details(3)]);
        let _details = server
            .mock(
                "GET",
                mockito::Matcher::Regex("exercises/details".to_string()),
            )
            .with_body(serde_json::to_string(&body).unwrap())
            .create();
        let _missing = server
            .mock(
                "GET",
                mockito::Matcher::Regex("core/exercises/1/download".to_string()),
            )
            .with_status(404)
            .create();
        let _server_error = server
            .mock(
                "GET",
                mockito::Matcher::Regex("core/exercises/2/download".to_string()),
            )
            .with_status(500)
            .create();
        let _broken = server
            .mock(
                "GET",
                mockito::Matcher::Regex("core/exercises/3/download".to_string()),
            )
            .with_body("not a zip")
            .create();

        let TmcDownloadResult::Failure { mut failed, .. } =
            download_or_update_course_exercises(&client, projects_dir.path(), &[1, 2, 3], true)
                .unwrap()
        else {
            panic!("the downloads should have failed");
        };
        failed.sort_by_key(|(download, ..)| download.id);
        let failures = failed
            .iter()
            .map(|(download, _, failure)| (download.id, *failure))
            .collect::<Vec<_>>();
        assert_eq!(
            failures,
            [
                (1, TmcDownloadFailure::NotFound),
                (2, TmcDownloadFailure::ServerError),
                (3, TmcDownloadFailure::ExtractFailed),
            ]
        );
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

        // The first download target of each course; "some course" records no id yet.
        let mut course_id_lookups = vec![];
        for (exercise_id, course_id) in [(1, 11), (3, 22)] {
            course_id_lookups.push(
                server
                    .mock(
                        "GET",
                        format!("/api/v8/core/exercises/{exercise_id}").as_str(),
                    )
                    .match_query(mockito::Matcher::Any)
                    .with_body(
                        serde_json::json!({
                            "course_name": "unused",
                            "course_id": course_id,
                            "code_review_requests_enabled": false,
                            "run_tests_locally_action_enabled": true,
                            "exercise_name": "unused",
                            "exercise_id": exercise_id,
                            "unlocked_at": null,
                            "deadline": null,
                            "submissions": [],
                        })
                        .to_string(),
                    )
                    .expect(1)
                    .create(),
            );
        }

        let res =
            download_or_update_course_exercises(&client, projects_dir.path(), &exercises, false)
                .unwrap();
        for lookup in course_id_lookups {
            lookup.assert();
        }
        let projects_config = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert_eq!(
            projects_config.tmc_courses["some course"].course_id,
            Some(11)
        );
        assert_eq!(
            projects_config.tmc_courses["another course"].course_id,
            Some(22)
        );
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

        // exercises from a 0.40–0.42 download stay put; new ones use the legacy layout
        assert_eq!(
            e1.path,
            projects_dir
                .path()
                .join("tmc/some course/on disk exercise with update and submission")
        );
        assert_eq!(
            e3.path,
            projects_dir
                .path()
                .join("another course/not on disk exercise with submission")
        );
        assert!(
            projects_dir
                .path()
                .join("another course/course_config.toml")
                .is_file()
        );

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
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");

        // The check must batch via the course-exercises route; only it is mocked,
        // so a per-exercise regression fails.
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
        assert_eq!(updates.updated, vec![exercise_id]);
        assert!(updates.removed.is_empty());
    }

    fn mooc_http_error(status: u16, obsolete_client: bool) -> Box<mooc::MoocClientError> {
        mooc_keyed_http_error(status, obsolete_client, None)
    }

    fn mooc_keyed_http_error(
        status: u16,
        obsolete_client: bool,
        message_key: Option<&str>,
    ) -> Box<mooc::MoocClientError> {
        Box::new(mooc::MoocClientError::HttpError {
            url: "https://courses.mooc.fi/api/v0/exercise-services/client/courses/1/exercises"
                .parse()
                .unwrap(),
            status: oauth2::http::StatusCode::from_u16(status).unwrap(),
            error: String::new(),
            obsolete_client,
            message_key: message_key.map(str::to_string),
            meta: Default::default(),
        })
    }

    #[test]
    fn course_refusal_kind_names_403_and_404() {
        assert_eq!(
            CourseRefusalKind::of(&MoocAuthFailure::Other(mooc_http_error(403, false))),
            Some(CourseRefusalKind::Forbidden)
        );
        assert_eq!(
            CourseRefusalKind::of(&MoocAuthFailure::Other(mooc_keyed_http_error(
                403,
                false,
                Some(mooc::INSUFFICIENT_SCOPE_MESSAGE_KEY)
            ))),
            Some(CourseRefusalKind::InsufficientScope)
        );
        assert_eq!(
            CourseRefusalKind::of(&MoocAuthFailure::Other(mooc_http_error(404, false))),
            Some(CourseRefusalKind::NotFound)
        );
    }

    #[test]
    fn course_refusal_kind_leaves_session_failures_unnamed() {
        for failure in [
            MoocAuthFailure::Other(mooc_http_error(401, false)),
            MoocAuthFailure::Other(mooc_http_error(426, false)),
            MoocAuthFailure::Other(mooc_http_error(429, false)),
            MoocAuthFailure::Other(mooc_http_error(500, false)),
            MoocAuthFailure::Other(mooc_http_error(503, false)),
            MoocAuthFailure::Other(mooc_http_error(403, true)),
            MoocAuthFailure::Permanent(mooc_http_error(403, false)),
            MoocAuthFailure::Permanent(mooc_http_error(404, false)),
            MoocAuthFailure::Other(Box::new(mooc::MoocClientError::NotAuthenticated {
                detail: None,
                meta: Default::default(),
            })),
        ] {
            assert_eq!(CourseRefusalKind::of(&failure), None, "{failure:?}");
        }
    }

    #[test]
    fn check_mooc_exercise_updates_reports_a_course_whose_progress_is_refused() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        write_mooc_course_config(projects_dir.path(), "old checksum");
        add_second_mooc_exercise(projects_dir.path());
        let course_id = ProjectsConfig::load(projects_dir.path())
            .unwrap()
            .mooc_courses
            .values()
            .next()
            .unwrap()
            .course_id;

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_body("[]")
            .create();
        let _progress = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/progress".to_string(),
                ),
            )
            .with_status(404)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(
            updates,
            MoocExerciseUpdates {
                updated: vec![],
                removed: vec![],
                unchecked_courses: vec![UncheckedMoocCourse {
                    course_id,
                    kind: CourseRefusalKind::NotFound,
                }],
            }
        );
    }

    #[test]
    fn check_mooc_exercise_updates_reports_exercises_removed_from_course() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");
        let removed_id = add_second_mooc_exercise(projects_dir.path());

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
        let _progress = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/progress".to_string(),
                ),
            )
            .with_body(progress_with_closed_chapter_json(&[]))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(
            updates,
            MoocExerciseUpdates {
                updated: vec![exercise_id],
                removed: vec![removed_id],
                unchecked_courses: vec![],
            }
        );
    }

    #[test]
    fn check_mooc_exercise_updates_does_not_report_closed_chapter_exercises_as_removed() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "same checksum");
        let closed_id = add_second_mooc_exercise(projects_dir.path());

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
        let _progress = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/progress".to_string(),
                ),
            )
            .with_body(progress_with_closed_chapter_json(&[closed_id]))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let updates = check_mooc_exercise_updates(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(updates, MoocExerciseUpdates::default());
        assert!(
            projects_dir
                .path()
                .join("mooc/my-course/ex-2/src/main.py")
                .is_file()
        );
    }

    #[test]
    fn update_mooc_exercises_leaves_closed_chapter_exercises_in_place() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "same checksum");
        let closed_id = add_second_mooc_exercise(projects_dir.path());

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
        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();

        assert!(result.downloaded.is_empty());
        let contents = file_util::read_file_to_string(
            projects_dir.path().join("mooc/my-course/ex-2/src/main.py"),
        )
        .unwrap();
        assert_eq!(contents, "student code");
        let config = ProjectsConfig::load(projects_dir.path()).unwrap();
        let course = config.mooc_courses.values().next().unwrap();
        assert_eq!(course.exercises[&closed_id].checksum, "old checksum");
    }

    #[test]
    fn update_mooc_exercises_skips_a_course_that_is_no_longer_served() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        write_mooc_course_config(projects_dir.path(), "old checksum");

        let _m = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"/api/v0/exercise-services/client/courses/[^/]+/exercises".to_string(),
                ),
            )
            .with_status(404)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();

        assert!(result.downloaded.is_empty());
    }

    #[test]
    fn checks_mooc_exercise_updates_no_change() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
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
        assert_eq!(updates, MoocExerciseUpdates::default());
    }

    #[test]
    fn updates_mooc_exercises_extracts_tar_zst() {
        // Regression: the archive comes from the public spec's `stub_download_url`
        // (no `exercises/{id}/download` route exists) and is `.tar.zst`, so
        // extraction must use `TarZstd`, not `Zip`.
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
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

    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    #[test]
    fn update_mooc_exercises_replaces_exercise_whose_language_changed() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");
        let exercise_dir = projects_dir.path().join("mooc/my-course/ex-1");
        file_to(&exercise_dir, "build.xml", "<project/>");
        file_to(&exercise_dir, "nbproject/project.properties", "");
        file_to(&exercise_dir, "src/Main.java", "class Main {}");
        file_to(&exercise_dir, "test/MainTest.java", "class MainTest {}");
        assert_eq!(
            PluginType::from_exercise(&exercise_dir).unwrap(),
            PluginType::Ant
        );

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
        let archive = make_tar_zst(&[
            ("R/main.R", b"f <- function() 1"),
            ("tests/testthat/test-main.R", b"test_that('f', {})"),
        ]);
        let _archive_mock = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(result.downloaded.len(), 1);
        assert!(exercise_dir.join("R/main.R").is_file());
        assert!(exercise_dir.join("tests/testthat/test-main.R").is_file());
        for stale in ["build.xml", "nbproject", "src", "test"] {
            assert!(
                !exercise_dir.join(stale).exists(),
                "{stale} was left behind"
            );
        }
        assert_eq!(
            PluginType::from_exercise(&exercise_dir).unwrap(),
            PluginType::R
        );
    }

    #[test]
    fn update_mooc_exercises_removes_files_the_new_package_dropped() {
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
            write_mooc_course_config(projects_dir.path(), "old checksum");
        let exercise_dir = projects_dir.path().join("mooc/my-course/ex-1");
        file_to(&exercise_dir, "src/old.py", "student work");
        file_to(&exercise_dir, "test/__init__.py", "");
        file_to(&exercise_dir, "test/test_old.py", "old test");
        file_to(&exercise_dir, "tmc/__main__.py", "");
        file_to(&exercise_dir, ".vscode/settings.json", "{}");

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
        let archive = make_tar_zst(&[
            ("src/new.py", b"print('new')"),
            ("test/__init__.py", b""),
            ("test/test_new.py", b"new test"),
        ]);
        let _archive_mock = server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(archive)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = update_mooc_exercises(&client, &auth, projects_dir.path()).unwrap();

        assert_eq!(result.downloaded.len(), 1);
        assert_eq!(
            std::fs::read_to_string(exercise_dir.join("src/old.py")).unwrap(),
            "student work"
        );
        assert!(exercise_dir.join("src/new.py").is_file());
        assert!(exercise_dir.join("test/test_new.py").is_file());
        assert!(exercise_dir.join(".vscode/settings.json").is_file());
        assert!(!exercise_dir.join("test/test_old.py").exists());
        assert!(!exercise_dir.join("tmc").exists());
    }

    #[test]
    fn update_mooc_exercises_persists_refreshed_checksum() {
        // The new checksum must be persisted, or every later check re-reports the
        // update.
        init();
        let mut server = Server::new();

        let projects_dir = tempfile::tempdir().unwrap();
        let (_course_id, exercise_id, task_id) =
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
            updates.updated.is_empty(),
            "update re-reported after the refreshed checksum should have been persisted: {updates:?}"
        );
    }

    #[test]
    fn resets_mooc_exercise_over_local_dir() {
        // Reset replaces the contents rather than overlaying: leftover.txt
        // (absent from the stub) must be gone.
        init();
        let mut server = Server::new();
        let exercise_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let parent = tempfile::tempdir().unwrap();
        let exercise_dir = parent.path().join("exercise");
        file_to(&exercise_dir, "src/main.py", b"stale student code");
        file_to(&exercise_dir, "leftover.txt", b"should be gone");
        #[cfg(unix)]
        let inode_before = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&exercise_dir).unwrap().ino()
        };

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
        reset_mooc_exercise(&client, &auth, exercise_id, &exercise_dir, false).unwrap();

        let main = file_util::read_file_to_string(exercise_dir.join("src/main.py")).unwrap();
        assert_eq!(main, "print('fresh stub')");
        assert!(!exercise_dir.join("leftover.txt").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(&exercise_dir).unwrap().ino(),
                inode_before,
                "the exercise directory itself must survive, so editor watchers keep working"
            );
        }
        assert_eq!(
            std::fs::read_dir(parent.path()).unwrap().count(),
            1,
            "staging directories must be cleaned up"
        );
    }

    /// Whether `dir`'s permissions stop this process from creating entries in it; root
    /// ignores them, which makes a permission-based failure test meaningless.
    #[cfg(unix)]
    fn is_write_protected(dir: &Path) -> bool {
        std::fs::create_dir(dir.join("probe")).is_err()
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn replace_dir_contents_restores_the_old_entries_when_the_new_ones_cannot_move_in() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("exercise");
        file_to(&target, "src/main.py", b"student code");
        file_to(&target, "notes.txt", b"my notes");
        let staged = staging_dir_beside(&target).unwrap();
        file_to(staged.path(), "src/main.py", b"stub");
        // Entries of a read-only directory cannot be renamed away.
        set_mode(staged.path(), 0o555);
        if !is_write_protected(staged.path()) {
            set_mode(staged.path(), 0o755);
            return;
        }

        let result = replace_dir_contents(&target, staged.path());
        set_mode(staged.path(), 0o755);

        assert!(result.is_err());
        assert_eq!(
            file_util::read_file_to_string(target.join("src/main.py")).unwrap(),
            "student code"
        );
        assert_eq!(
            file_util::read_file_to_string(target.join("notes.txt")).unwrap(),
            "my notes"
        );
        drop(staged);
        assert_eq!(
            std::fs::read_dir(parent.path()).unwrap().count(),
            1,
            "the directory holding the old entries must be cleaned up after a rollback"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replace_dir_contents_leaves_the_target_alone_when_its_entries_cannot_move_out() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("exercise");
        file_to(&target, "src/main.py", b"student code");
        let staged = staging_dir_beside(&target).unwrap();
        file_to(staged.path(), "src/main.py", b"stub");
        set_mode(&target, 0o555);
        if !is_write_protected(&target) {
            set_mode(&target, 0o755);
            return;
        }

        let result = replace_dir_contents(&target, staged.path());
        set_mode(&target, 0o755);

        assert!(result.is_err());
        assert_eq!(
            file_util::read_file_to_string(target.join("src/main.py")).unwrap(),
            "student code"
        );
    }

    #[test]
    fn replace_dir_contents_moves_the_staged_dir_into_a_missing_target() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("exercise");
        let staged = staging_dir_beside(&target).unwrap();
        file_to(staged.path(), "src/main.py", b"stub");

        replace_dir_contents(&target, staged.path()).unwrap();

        assert_eq!(
            file_util::read_file_to_string(target.join("src/main.py")).unwrap(),
            "stub"
        );
    }

    fn maven_archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        for (name, contents) in files {
            zw.start_file(*name, SimpleFileOptions::default()).unwrap();
            zw.write_all(contents.as_bytes()).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    fn staging_dir_names(parent: &Path) -> Vec<OsString> {
        dir_entry_names(parent)
            .unwrap()
            .into_iter()
            .filter(|n| {
                let n = n.to_string_lossy();
                n.starts_with(STAGING_DIR_PREFIX) || n.starts_with(REPLACED_DIR_PREFIX)
            })
            .collect()
    }

    #[test]
    fn extract_update_swaps_in_the_new_version_and_keeps_student_files() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let exercise = parent.path().join("ex");
        file_to(&exercise, "pom.xml", b"old");
        file_to(&exercise, "src/main/java/File.java", b"student work");
        file_to(&exercise, "src/test/java/Stale.java", b"stale");
        file_to(&exercise, ".venv/keep", b"env");
        let archive = maven_archive(&[
            ("pom.xml", "new"),
            ("src/main/java/File.java", "template"),
            ("src/test/java/FileTest.java", "test"),
        ]);

        extract_update(archive, &exercise, Compression::Zip).unwrap();

        let read = |p: &str| file_util::read_file_to_string(exercise.join(p)).unwrap();
        assert_eq!(read("pom.xml"), "new");
        assert_eq!(read("src/main/java/File.java"), "student work");
        assert_eq!(read("src/test/java/FileTest.java"), "test");
        assert_eq!(read(".venv/keep"), "env");
        assert!(!exercise.join("src/test/java/Stale.java").exists());
        assert!(staging_dir_names(parent.path()).is_empty());
    }

    #[test]
    fn extract_update_leaves_the_exercise_alone_when_extraction_fails() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let exercise = parent.path().join("ex");
        file_to(&exercise, "pom.xml", b"old");
        file_to(&exercise, "src/main/java/File.java", b"student work");

        let result = extract_update(b"not an archive".to_vec(), &exercise, Compression::Zip);

        assert!(result.is_err());
        let read = |p: &str| file_util::read_file_to_string(exercise.join(p)).unwrap();
        assert_eq!(read("pom.xml"), "old");
        assert_eq!(read("src/main/java/File.java"), "student work");
        assert!(staging_dir_names(parent.path()).is_empty());
    }

    #[test]
    fn extract_update_downloads_a_missing_exercise_whole() {
        init();
        let parent = tempfile::tempdir().unwrap();
        let exercise = parent.path().join("course/ex");
        let archive = maven_archive(&[("pom.xml", "new"), ("src/main/java/File.java", "t")]);

        extract_update(archive, &exercise, Compression::Zip).unwrap();

        assert_eq!(
            file_util::read_file_to_string(exercise.join("pom.xml")).unwrap(),
            "new"
        );
    }

    #[test]
    fn staging_sweep_removes_only_old_staging_dirs() {
        init();
        let parent = tempfile::tempdir().unwrap();
        file_to(parent.path(), ".tmc-staging-abc/left", b"x");
        file_to(parent.path(), ".tmc-replaced-abc/student", b"x");
        file_to(parent.path(), "exercise/file", b"x");

        sweep_stale_staging_dirs(parent.path(), std::time::Duration::from_secs(3600));
        assert!(parent.path().join(".tmc-staging-abc").exists());

        sweep_stale_staging_dirs(parent.path(), std::time::Duration::ZERO);
        assert!(!parent.path().join(".tmc-staging-abc").exists());
        assert!(parent.path().join(".tmc-replaced-abc/student").exists());
        assert!(parent.path().join("exercise/file").exists());
    }

    #[test]
    fn download_old_submission_keeps_the_work_when_the_download_fails() {
        init();
        let mut server = Server::new();
        let exercise = tempfile::tempdir().unwrap();
        file_to(exercise.path(), "src/main/java/File.java", b"student work");
        let template = maven_archive(&[("pom.xml", "t"), ("src/main/java/File.java", "t")]);
        let _template = server
            .mock(
                "GET",
                mockito::Matcher::AllOf(vec![
                    mockito::Matcher::Regex("exercises".to_string()),
                    mockito::Matcher::Regex("download".to_string()),
                ]),
            )
            .with_body(template)
            .create();
        let _submission = server
            .mock("GET", mockito::Matcher::Regex("submission".to_string()))
            .with_status(500)
            .create();
        let client = mock_testmycode_client(&server);

        let result = download_old_submission(&client, 1, exercise.path(), 2, false);

        assert!(result.is_err());
        assert_eq!(
            file_util::read_file_to_string(exercise.path().join("src/main/java/File.java"))
                .unwrap(),
            "student work"
        );
    }

    fn moved_dir_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("old");
        let target = parent.path().join("new");
        file_to(&source, "course/ex/src/main.py", b"code");
        file_to(&source, "course/course_config.toml", b"cfg");
        (parent, source, target)
    }

    #[test]
    fn move_dir_renames_and_records_the_move() {
        init();
        let (_parent, source, target) = moved_dir_fixture();
        std::fs::create_dir(&target).unwrap();
        let mut recorded = false;
        move_dir(&source, &target, || {
            recorded = true;
            Ok(())
        })
        .unwrap();
        assert!(recorded);
        assert!(!source.exists());
        assert!(target.join("course/ex/src/main.py").is_file());
    }

    #[test]
    fn move_dir_copy_fallback_records_the_move_before_deleting_the_old_dir() {
        init();
        let (_parent, source, target) = moved_dir_fixture();
        move_dir_with(&source, &target, false, || {
            assert!(source.join("course/ex/src/main.py").is_file());
            assert!(target.join("course/ex/src/main.py").is_file());
            Ok(())
        })
        .unwrap();
        assert!(!source.exists());
        assert_eq!(
            file_util::read_file_to_string(target.join("course/course_config.toml")).unwrap(),
            "cfg"
        );
    }

    #[test]
    fn move_dir_copy_fallback_keeps_the_old_dir_when_recording_fails() {
        init();
        let (_parent, source, target) = moved_dir_fixture();
        let result = move_dir_with(&source, &target, false, || {
            Err(LangsError::NoProjectExercise)
        });
        assert!(result.is_err());
        assert!(source.join("course/ex/src/main.py").is_file());
        assert!(dir_entry_names(&target).unwrap().is_empty());
    }

    #[test]
    fn move_dir_rename_is_undone_when_recording_fails() {
        init();
        let (_parent, source, target) = moved_dir_fixture();
        let result = move_dir(&source, &target, || Err(LangsError::NoProjectExercise));
        assert!(result.is_err());
        assert!(source.join("course/ex/src/main.py").is_file());
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn move_dir_copy_fallback_keeps_symlinks_as_symlinks() {
        init();
        let (_parent, source, target) = moved_dir_fixture();
        file_to(&source, "course/ex/.venv/lib/site.py", b"x");
        std::os::unix::fs::symlink("lib", source.join("course/ex/.venv/lib64")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/python3", source.join("course/ex/.venv/python"))
            .unwrap();

        move_dir_with(&source, &target, false, || Ok(())).unwrap();

        let venv = target.join("course/ex/.venv");
        assert!(venv.join("lib64").is_symlink());
        assert_eq!(
            std::fs::read_link(venv.join("lib64")).unwrap(),
            Path::new("lib")
        );
        assert!(venv.join("python").is_symlink());
        assert!(!source.exists());
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
        // A changed exercise is downloaded, an unchanged one skipped, a browser-only
        // and an unknown one reported failed. Course context comes from scanning
        // enrolled courses (no exercise -> course endpoint).
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
            MoocExerciseSource::EnrolledCourses,
        )
        .unwrap();

        assert_eq!(result.downloaded.len(), 1, "one exercise downloaded");
        // results are keyed by the requested exercise id, not the editor task id
        assert_eq!(result.downloaded[0].exercise_id, ex_new);
        assert_eq!(result.skipped.len(), 1, "one exercise skipped");
        assert_eq!(result.skipped[0].exercise_id, ex_skip);
        let failed = result.failed.expect("two failures");
        assert_eq!(failed.len(), 2, "browser + missing exercises fail");
        let failures = failed
            .iter()
            .map(|(d, _, failure)| (d.exercise_id, *failure))
            .collect::<HashMap<_, _>>();
        assert_eq!(failures[&ex_browser], MoocDownloadFailure::BrowserOnly);
        assert_eq!(failures[&ex_missing], MoocDownloadFailure::NotFound);

        let extracted = projects_dir
            .path()
            .join("mooc/course/new-exercise/src/main.py");
        assert_eq!(
            file_util::read_file_to_string(&extracted).unwrap(),
            "print('new')"
        );

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
            MoocExerciseSource::Course(course_id),
        )
        .unwrap();

        assert_eq!(result.downloaded.len(), 1);
        assert_eq!(result.downloaded[0].exercise_id, ex_new);
        let reloaded = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert!(reloaded.get_mooc_exercise(course_id, ex_new).is_some());
    }

    #[test]
    fn download_or_update_reports_closed_chapter_exercises_as_not_open_yet() {
        init();
        let mut server = Server::new();

        let course_id = Uuid::new_v4();
        let closed_id = Uuid::new_v4();
        let projects_dir = tempfile::tempdir().unwrap();

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
            .create();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
            )
            .with_body("[]")
            .create();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/progress").as_str(),
            )
            .with_body(progress_with_closed_chapter_json(&[closed_id]))
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = download_or_update_mooc_course_exercises(
            &client,
            &auth,
            projects_dir.path(),
            &[closed_id],
            MoocExerciseSource::Course(course_id),
        )
        .unwrap();

        let failed = result.failed.unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0.exercise_id, closed_id);
        assert_eq!(failed[0].2, MoocDownloadFailure::ClosedChapter);
        assert_eq!(
            failed[0].1,
            vec![format!(
                "Exercise {closed_id} is in chapter 2 \"Later\", which opens at 2030-01-01 00:00:00 UTC"
            )]
        );
    }

    #[test]
    fn a_batch_of_same_named_exercises_gets_one_folder_each() {
        init();
        let mut server = Server::new();
        let course_id = Uuid::new_v4();
        let projects_dir = tempfile::tempdir().unwrap();
        let exercise_ids: Vec<Uuid> = (0..6).map(|_| Uuid::new_v4()).collect();
        let stub_url = format!("{}/files/stub.tar.zst", server.url());

        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "id": course_id, "slug": "course", "name": "../Course",
                    "description": null, "organization_name": "org",
                })
                .to_string(),
            )
            .create();
        let slides: Vec<_> = exercise_ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                serde_json::json!({
                    "slide_id": Uuid::new_v4(), "exercise_id": id, "course_id": course_id,
                    "exercise_name": if i % 2 == 0 { "U15 Dup" } else { "u15-dup" },
                    "exercise_order_number": i,
                    "tasks": [{
                        "task_id": Uuid::new_v4(), "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "editor", "archive_name": "s.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": ["src/main.py"], "checksum": "sum"
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                })
            })
            .collect();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
            )
            .with_body(serde_json::Value::Array(slides).to_string())
            .create();
        server
            .mock("GET", "/files/stub.tar.zst")
            .with_body(make_tar_zst(&[("src/main.py", b"print(1)")]))
            .expect(6)
            .create();

        let client = mock_mooc_client(&server);
        let auth = mock_mooc_auth(&server);
        let result = download_or_update_mooc_course_exercises(
            &client,
            &auth,
            projects_dir.path(),
            &exercise_ids,
            MoocExerciseSource::Course(course_id),
        )
        .unwrap();

        assert_eq!(result.downloaded.len(), 6, "{result:?}");
        let folders: HashSet<_> = result.downloaded.iter().map(|d| d.path.clone()).collect();
        assert_eq!(folders.len(), 6);
        for folder in &folders {
            assert!(folder.starts_with(projects_dir.path().join("mooc/course")));
            assert!(folder.join("src/main.py").is_file());
        }
        let reloaded = ProjectsConfig::load(projects_dir.path()).unwrap();
        assert_eq!(reloaded.mooc_courses[&course_id].exercises.len(), 6);
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
        // tmc-server rejecting a mooc token says nothing about its validity at
        // courses.mooc.fi, so the 401 path must find nothing to delete.
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
