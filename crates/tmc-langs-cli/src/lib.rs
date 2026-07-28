#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! CLI client for TMC.

pub mod app;
pub mod error;
pub mod output;

use self::{
    error::{InvalidTokenError, SandboxTestError},
    output::{CliOutput, DataKind, Kind, OutputData, OutputResult, Status},
};
use crate::app::{Cli, Locale};
use anyhow::{Context, Result};
use app::{Command, Mooc, MoocCommand, Settings, SettingsCommand, TestMyCode, TestMyCodeCommand};
use base64::Engine;
use clap::{CommandFactory, error::ErrorKind};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    env,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tmc_langs::{
    CommandError, Compression, DownloadOrUpdateTmcCourseExercisesResult, LangsError, Language,
    StyleValidationResult, TmcConfig, TmcDownloadResult, TmcProjectYml, UpdatedExercise,
    file_util::{self, Lock, LockOptions},
    mooc::{self, MoocClient, MoocClientError},
    progress_reporter,
    tmc::{TestMyCodeClient, TestMyCodeClientError, request::FeedbackAnswer},
};
use tmc_langs_util::deserialize;
use uuid::Uuid;

pub enum ParsingResult {
    Ok(Cli),
    Help(clap::Error),
    Version(clap::Error),
    Err(CliOutput),
}

pub fn map_parsing_result(result: Result<Cli, clap::Error>) -> ParsingResult {
    match result {
        Ok(cli) => ParsingResult::Ok(cli),
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => ParsingResult::Help(e),
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayVersion => ParsingResult::Version(e),
        Err(e) => {
            // CLI was called incorrectly
            let e = anyhow::Error::from(e).context("Failed to parse arguments");
            let causes: Vec<String> = e.chain().map(|e| format!("Caused by: {e}")).collect();
            let output = CliOutput::OutputData(Box::new(OutputData {
                status: Status::Finished,
                message: format!("{e:?}"), // debug formatting to print backtrace from anyhow
                result: OutputResult::Error,
                data: Some(DataKind::Error {
                    kind: Kind::Generic,
                    trace: causes,
                }),
            }));
            ParsingResult::Err(output)
        }
    }
}

#[derive(Debug)]
pub struct CliError {
    pub output: Box<CliOutput>,
    pub sandbox_path: Option<PathBuf>,
}

pub fn run(cli: Cli) -> Result<CliOutput, CliError> {
    match run_app(cli) {
        Ok(output) => Ok(output),
        Err(e) => {
            // error handling
            let causes: Vec<String> = e.chain().map(|e| format!("Caused by: {e}")).collect();
            let message = error_message_special_casing(&e);
            let kind = solve_error_kind(&e);
            let sandbox_path = check_sandbox_err(&e);
            let output = CliOutput::OutputData(Box::new(OutputData {
                status: Status::Finished,
                message,
                result: OutputResult::Error,
                data: Some(DataKind::Error {
                    kind,
                    trace: causes,
                }),
            }));
            Err(CliError {
                output: Box::new(output),
                sandbox_path,
            })
        }
    }
}

/// Goes through the error chain and checks for special error types that should be indicated by the Kind.
fn solve_error_kind(e: &anyhow::Error) -> Kind {
    for cause in e.chain() {
        // check for invalid token
        if cause.downcast_ref::<InvalidTokenError>().is_some() {
            return Kind::InvalidToken;
        }

        // check for tmc client errors
        match cause.downcast_ref::<TestMyCodeClientError>() {
            Some(TestMyCodeClientError::HttpError {
                url: _,
                status,
                error: _,
                obsolete_client,
            }) => {
                if *obsolete_client {
                    return Kind::ObsoleteClient;
                }
                if status.as_u16() == 403 {
                    return Kind::Forbidden;
                }
                if status.as_u16() == 401 {
                    return Kind::NotLoggedIn;
                }
            }
            Some(TestMyCodeClientError::NotAuthenticated) => {
                return Kind::NotLoggedIn;
            }
            Some(TestMyCodeClientError::ConnectionError { .. }) => {
                return Kind::ConnectionError;
            }
            _ => {}
        }

        // NB: mooc errors travel the chain as `Box<MoocClientError>`, so a bare
        // downcast never matches; the boxed form must be checked too (same as the
        // 401 handler in `run_mooc`).
        if let Some(kind) = cause
            .downcast_ref::<MoocClientError>()
            .or_else(|| {
                cause
                    .downcast_ref::<Box<MoocClientError>>()
                    .map(Box::as_ref)
            })
            .and_then(mooc_error_kind)
        {
            return kind;
        }

        // A refresh failure surfaces wrapped in `LangsError::MoocClient`, a
        // `#[error(transparent)]` variant whose `source()` delegates *past* the
        // `MoocClientError` (to the inner reqwest error, or to nothing for an
        // HTTP status error), so the downcasts above never see it. Unwrap that
        // case explicitly so a transient refresh (5xx / connection error) still
        // maps to `connection-error` rather than the opaque `generic`.
        if let Some(LangsError::MoocClient(mooc_err)) = cause.downcast_ref::<LangsError>() {
            if let Some(kind) = mooc_error_kind(mooc_err) {
                return kind;
            }
        }

        // Every individual mooc network call now goes through
        // `MoocAuth::call`, which surfaces a failed call as `MoocAuthFailure`
        // (see `mooc_credentials.rs`). Unwrap it explicitly rather than
        // relying on exactly how its `#[source]` link shapes the chain.
        if let Some(mooc_auth_err) = cause.downcast_ref::<tmc_langs::MoocAuthFailure>() {
            let inner = match mooc_auth_err {
                tmc_langs::MoocAuthFailure::Permanent(e) | tmc_langs::MoocAuthFailure::Other(e) => {
                    e.as_ref()
                }
            };
            if let Some(kind) = mooc_error_kind(inner) {
                return kind;
            }
        }
    }

    Kind::Generic
}

/// Maps a [`MoocClientError`] to the CLI error [`Kind`] it should surface, if
/// any. Shared so the same classification applies whether the error reaches the
/// anyhow chain bare, boxed, or wrapped in `LangsError::MoocClient` (as a
/// proactive or on-401 token-refresh failure does).
fn mooc_error_kind(err: &MoocClientError) -> Option<Kind> {
    match err {
        MoocClientError::HttpError {
            status,
            obsolete_client,
            message_key,
            ..
        } => {
            // `message_key` is the backend's stable identifier for a controlled
            // error; prefer it over the raw status where present.
            if *obsolete_client || message_key.as_deref() == Some("obsolete_client") {
                Some(Kind::ObsoleteClient)
            } else if message_key.as_deref() == Some("not_enrolled") {
                Some(Kind::NotEnrolled)
            } else if status.as_u16() == 403 {
                Some(Kind::Forbidden)
            } else if status.as_u16() == 401 {
                Some(Kind::NotLoggedIn)
            } else if status.is_server_error() {
                // A 5xx is a transient server-side failure (e.g. a refresh that
                // hit a flaky backend); surface it as a retryable connection
                // error rather than deleting credentials or reporting `generic`.
                Some(Kind::ConnectionError)
            } else {
                None
            }
        }
        MoocClientError::NotAuthenticated => Some(Kind::NotLoggedIn),
        // A denied or expired device authorization is a failed login, not a hard
        // error: surface it as "not logged in".
        MoocClientError::DeviceAccessDenied | MoocClientError::DeviceCodeExpired => {
            Some(Kind::NotLoggedIn)
        }
        // The refresh token was permanently rejected (invalid/revoked/expired):
        // the stored credentials are gone, so the user must log in again.
        MoocClientError::RefreshTokenRejected { .. } => Some(Kind::NotLoggedIn),
        MoocClientError::ConnectionError { .. } => Some(Kind::ConnectionError),
        _ => None,
    }
}

/// Goes through the error chain and returns the specialized error message, if any.
fn error_message_special_casing(e: &anyhow::Error) -> String {
    for cause in e.chain() {
        // command not found errors are special cased to notify the user that they may need to install additional software
        if let Some(cnf) = cause.downcast_ref::<CommandError>() {
            return cnf.to_string();
        }
    }
    e.to_string()
}

/// Goes through the error chain and returns the error output file path if a sandbox test error is found
fn check_sandbox_err(e: &anyhow::Error) -> Option<PathBuf> {
    for cause in e.chain() {
        if let Some(SandboxTestError {
            path: Some(path), ..
        }) = cause.downcast_ref::<SandboxTestError>()
        {
            return Some(path.clone());
        }
    }
    None
}

fn run_app(cli: Cli) -> Result<CliOutput> {
    let output = match cli.command {
        Command::Checkstyle {
            exercise_path,
            locale: Locale(locale),
            output_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let check_result =
                run_checkstyle_write_results(&exercise_path, output_path.as_deref(), locale)?;
            CliOutput::finished_with_data("ran checkstyle", DataKind::Validation(check_result))
        }

        Command::Clean { exercise_path } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Write)?;
            let _guard = lock.lock()?;

            tmc_langs::clean(&exercise_path)?;
            CliOutput::finished(format!("cleaned exercise at {}", exercise_path.display()))
        }

        Command::CompressProject {
            exercise_path,
            output_path,
            compression,
            deterministic,
            naive,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let hash = tmc_langs::compress_project_to_with_hash(
                &exercise_path,
                &output_path,
                compression,
                deterministic,
                naive,
            )?;
            CliOutput::finished_with_data(
                format!(
                    "compressed project from {} to {}",
                    exercise_path.display(),
                    output_path.display()
                ),
                DataKind::CompressedProjectHash(hash),
            )
        }

        Command::Tmc(tmc) => run_tmc(tmc)?,

        Command::Mooc(mooc) => run_mooc(mooc)?,

        Command::ExtractProject {
            archive_path,
            output_path,
            compression,
            naive,
        } => {
            let mut archive_lock = Lock::file(&archive_path, LockOptions::Read)?;
            let mut archive_guard = archive_lock.lock()?;

            let mut data = vec![];
            archive_guard.get_file_mut().read_to_end(&mut data)?;

            tmc_langs::extract_project(Cursor::new(data), &output_path, compression, true, naive)?;

            CliOutput::finished(format!(
                "extracted project from {} to {}",
                archive_path.display(),
                output_path.display()
            ))
        }

        Command::FastAvailablePoints { exercise_path } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let points = tmc_langs::get_available_points(&exercise_path)?;
            CliOutput::finished_with_data(
                format!("found {} available points", points.len()),
                DataKind::AvailablePoints(points),
            )
        }

        Command::FindExercises {
            search_path,
            output_path,
        } => {
            let mut lock = Lock::dir(&search_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let exercises =
                tmc_langs::find_exercise_directories(&search_path).with_context(|| {
                    format!(
                        "Failed to find exercise directories in {}",
                        search_path.display(),
                    )
                })?;
            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&exercises, &output_path, cli.pretty, None)?;
            }
            CliOutput::finished_with_data(
                format!("found exercises at {}", search_path.display()),
                DataKind::Exercises(exercises),
            )
        }

        Command::GetExercisePackagingConfiguration {
            exercise_path,
            output_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let config = tmc_langs::get_exercise_packaging_configuration(&exercise_path)
                .with_context(|| {
                    format!(
                        "Failed to get exercise packaging configuration for exercise at {}",
                        exercise_path.display(),
                    )
                })?;
            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&config, &output_path, cli.pretty, None)?;
            }
            CliOutput::finished_with_data(
                format!(
                    "created exercise packaging config from {}",
                    exercise_path.display(),
                ),
                DataKind::ExercisePackagingConfiguration(config),
            )
        }

        Command::ListLocalTmcCourseExercises {
            client_name,
            course_slug,
        } => {
            let local_exercises =
                tmc_langs::list_local_tmc_course_exercises(&client_name, &course_slug)?;

            CliOutput::finished_with_data(
                format!("listed local exercises for {course_slug}"),
                DataKind::LocalTmcExercises(local_exercises),
            )
        }

        Command::PrepareSolution {
            exercise_path,
            output_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            tmc_langs::prepare_solution(&exercise_path, &output_path).with_context(|| {
                format!(
                    "Failed to prepare solutions for exercise at {}",
                    exercise_path.display(),
                )
            })?;
            CliOutput::finished(format!(
                "prepared solutions for {} at {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::PrepareStub {
            exercise_path,
            output_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            tmc_langs::prepare_stub(&exercise_path, &output_path).with_context(|| {
                format!(
                    "Failed to prepare stubs for exercise at {}",
                    exercise_path.display(),
                )
            })?;
            CliOutput::finished(format!(
                "prepared stubs for {} at {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::PrepareSubmission {
            clone_path,
            output_format,
            output_path,
            stub_archive_path,
            stub_compression,
            submission_path,
            submission_compression,
            extract_submission_naively,
            tmc_param,
            no_archive_prefix,
        } => {
            let mut clone_lock = Lock::dir(&clone_path, file_util::LockOptions::Read)?;
            let _clone_guard = clone_lock.lock()?;

            // will contain for each key all the values with that key in a list
            let mut tmc_params_grouped = HashMap::new();
            for value in &tmc_param {
                let params: Vec<_> = value.split('=').collect();
                if params.len() != 2 {
                    app::Cli::command()
                        .error(
                            ErrorKind::ValueValidation,
                            "tmc-param values should contain a single '=' as a delimiter.",
                        )
                        .exit();
                }
                let key = params[0];
                let value = params[1];
                let entry = tmc_params_grouped.entry(key).or_insert_with(Vec::new);
                entry.push(value);
            }
            let mut tmc_params = tmc_langs::TmcParams::new();
            for (key, values) in tmc_params_grouped {
                if values.len() == 1 {
                    // 1-length lists are inserted as a string
                    tmc_params
                        .insert_string(key, values[0])
                        .context("invalid tmc-param key-value pair")?;
                } else {
                    tmc_params
                        .insert_array(key, values)
                        .context("invalid tmc-param key-value pair")?;
                }
            }

            let sandbox = tmc_langs::prepare_submission(
                tmc_langs::PrepareSubmission {
                    archive: &submission_path,
                    compression: submission_compression,
                    extract_naively: extract_submission_naively,
                },
                &output_path,
                no_archive_prefix,
                tmc_params,
                &clone_path,
                stub_archive_path.as_deref().map(|p| (p, stub_compression)),
                output_format,
            )?;
            CliOutput::finished_with_data(
                format!(
                    "prepared submission for {} at {}",
                    submission_path.display(),
                    output_path.display()
                ),
                DataKind::SubmissionSandbox(sandbox),
            )
        }

        Command::RefreshCourse {
            cache_path,
            cache_root,
            course_name,
            git_branch,
            source_url,
        } => {
            let refresh_result = tmc_langs::refresh_course(
                course_name.clone(),
                cache_path,
                source_url,
                git_branch,
                cache_root,
            )
            .with_context(|| format!("Failed to refresh course {course_name}"))?;
            CliOutput::finished_with_data(
                format!("refreshed course {course_name}"),
                DataKind::RefreshResult(refresh_result),
            )
        }

        Command::RunTests {
            checkstyle_output_path,
            exercise_path,
            locale,
            output_path,
            wait_for_secret,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let secret = if wait_for_secret {
                let mut s = String::new();
                io::stdin().read_line(&mut s)?;
                Some(s.trim().to_string())
            } else {
                None
            };

            let test_result = tmc_langs::run_tests(&exercise_path).with_context(|| {
                format!(
                    "Failed to run tests for exercise at {}",
                    exercise_path.display()
                )
            });

            let test_result = if env::var("TMC_SANDBOX").is_ok() {
                // in sandbox, wrap error to signal we want to write the output into a file
                test_result.map_err(|e| SandboxTestError {
                    path: output_path.clone(),
                    source: e,
                })?
            } else {
                // not in sandbox, just unwrap
                test_result?
            };

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&test_result, &output_path, cli.pretty, secret)?;
            }

            // todo: checkstyle results in stdout?
            if let Some(checkstyle_output_path) = checkstyle_output_path {
                let locale = locale
                    .expect("locale is required if checkstyle output path is given")
                    .0;

                run_checkstyle_write_results(
                    &exercise_path,
                    Some(&checkstyle_output_path),
                    locale,
                )?;
            }

            CliOutput::finished_with_data(
                format!("ran tests for {}", exercise_path.display()),
                DataKind::TestResult(test_result),
            )
        }

        Command::Settings(settings) => run_settings(settings)?,

        Command::ScanExercise {
            exercise_path,
            output_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let exercise_name = exercise_path.file_name().with_context(|| {
                format!(
                    "No file name found in exercise path {}",
                    exercise_path.display()
                )
            })?;

            let exercise_name = exercise_name.to_str().with_context(|| {
                format!("Exercise path's file name '{exercise_name:?}' was not valid UTF8")
            })?;

            let scan_result = tmc_langs::scan_exercise(&exercise_path, exercise_name.to_string())
                .with_context(|| {
                format!("Failed to scan exercise at {}", exercise_path.display())
            })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&scan_result, &output_path, cli.pretty, None)?;
            }

            CliOutput::finished_with_data(
                format!("scanned exercise at {}", exercise_path.display()),
                DataKind::ExerciseDesc(scan_result),
            )
        }

        // `main.rs` intercepts `Command::Schema` before the library runs, since
        // printing the raw schema needs stdout the library must not assume.
        // Reaching this arm means a library caller dispatched it directly.
        Command::Schema => {
            anyhow::bail!(
                "the `schema` subcommand is handled by the CLI binary, not the library `run()`"
            )
        }
    };
    Ok(output)
}

fn run_tmc(tmc: TestMyCode) -> Result<CliOutput> {
    let client_name = &tmc.client_name;
    let client_version = &tmc.client_version;
    let root_url = env::var("TMC_LANGS_TMC_ROOT_URL")
        .unwrap_or_else(|_| "https://tmc.mooc.fi/".to_string())
        .parse()
        .context("Invalid TMC root url")?;
    let mooc_root_url = mooc_root_url()?;
    let mooc_client_id = mooc_client_id();
    let (mut client, mut auth) = tmc_langs::init_testmycode_client_with_credentials(
        root_url,
        client_name,
        client_version,
        &mooc_root_url,
        &mooc_client_id,
    )?;

    match run_tmc_inner(tmc, &mut client, &mut auth) {
        Err(error) => {
            for cause in error.chain() {
                // check if the token was rejected and delete it if so
                if let Some(TestMyCodeClientError::HttpError { status, .. }) =
                    cause.downcast_ref::<TestMyCodeClientError>()
                {
                    if status.as_u16() == 401 {
                        // Only a stored TMC token is deleted here. A rejected
                        // mooc token stays put: tmc-server may simply not be
                        // accepting mooc tokens, which says nothing about the
                        // token's validity at courses.mooc.fi. That case falls
                        // through to the plain 401 mapping (`not-logged-in`)
                        // instead of claiming credentials were deleted.
                        if let Some(credentials) = auth.take_stored_tmc() {
                            log::error!("Received HTTP 401 error, deleting TMC credentials");
                            credentials.remove()?;
                            return Err(InvalidTokenError { source: error }.into());
                        }
                    }
                }
            }
            Err(error)
        }
        output => output,
    }
}

fn run_tmc_inner(
    tmc: TestMyCode,
    client: &mut TestMyCodeClient,
    auth: &mut tmc_langs::TestMyCodeAuth,
) -> Result<CliOutput> {
    let client_name = &tmc.client_name;
    let output = match tmc.command {
        TestMyCodeCommand::CheckExerciseUpdates => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let updated_exercises = tmc_langs::check_tmc_exercise_updates(client, &projects_dir)
                .context("Failed to check exercise updates")?
                .into_iter()
                .map(|id| UpdatedExercise { id })
                .collect::<Vec<_>>();

            CliOutput::finished_with_data(
                "updated exercises",
                DataKind::UpdatedExercises(updated_exercises),
            )
        }

        TestMyCodeCommand::DownloadModelSolution {
            exercise_id,
            target,
        } => {
            let mut output_lock = Lock::dir(&target, file_util::LockOptions::WriteTruncate)?;
            let _output_guard = output_lock.lock()?;

            client
                .download_model_solution(exercise_id, &target)
                .context("Failed to download model solution")?;
            CliOutput::finished("downloaded model solution")
        }

        TestMyCodeCommand::DownloadOldSubmission {
            submission_id,
            save_old_state,
            exercise_id,
            output_path,
        } => {
            let mut output_lock = Lock::dir(&output_path, file_util::LockOptions::Write)?;
            let output_guard = output_lock.lock()?;

            tmc_langs::download_old_submission(
                client,
                exercise_id,
                &output_path,
                submission_id,
                save_old_state,
            )?;
            drop(output_guard);
            output_lock.forget();
            CliOutput::finished("extracted project")
        }

        TestMyCodeCommand::DownloadOrUpdateCourseExercises {
            download_template,
            exercise_id: exercise_ids,
        } => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = match tmc_langs::download_or_update_course_exercises(
                client,
                &projects_dir,
                &exercise_ids,
                download_template,
            )? {
                TmcDownloadResult::Success {
                    downloaded,
                    skipped,
                } => DownloadOrUpdateTmcCourseExercisesResult {
                    downloaded,
                    skipped,
                    failed: None,
                },
                TmcDownloadResult::Failure {
                    downloaded,
                    skipped,
                    failed,
                } => DownloadOrUpdateTmcCourseExercisesResult {
                    downloaded,
                    skipped,
                    failed: Some(failed),
                },
            };
            CliOutput::finished_with_data(
                "downloaded or updated exercises",
                DataKind::TmcExerciseDownload(data),
            )
        }

        TestMyCodeCommand::GetCourseData { course_id } => {
            let data = tmc_langs::get_course_data(client, course_id)
                .context("Failed to get course data")?;
            CliOutput::finished_with_data(
                "fetched course data",
                DataKind::CombinedCourseData(Box::new(data)),
            )
        }

        TestMyCodeCommand::GetCourseDetails { course_id } => {
            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;
            CliOutput::finished_with_data(
                "fetched course details",
                DataKind::CourseDetails(details),
            )
        }

        TestMyCodeCommand::GetCourseExercises { course_id } => {
            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data(
                "fetched course exercises",
                DataKind::CourseExercises(exercises),
            )
        }

        TestMyCodeCommand::GetCourseSettings { course_id } => {
            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data("fetched course settings", DataKind::CourseData(settings))
        }

        TestMyCodeCommand::GetCourses { organization } => {
            let courses = client
                .list_courses(&organization)
                .context("Failed to get courses")?;
            CliOutput::finished_with_data("fetched courses", DataKind::Courses(courses))
        }

        TestMyCodeCommand::GetExerciseDetails { exercise_id } => {
            let course = client
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data(
                "fetched exercise details",
                DataKind::ExerciseDetails(course),
            )
        }

        TestMyCodeCommand::GetExerciseSubmissions { exercise_id } => {
            let submissions = client
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;
            CliOutput::finished_with_data(
                "fetched exercise submissions",
                DataKind::Submissions(submissions),
            )
        }

        TestMyCodeCommand::GetExerciseUpdates {
            course_id,
            exercise,
        } => {
            // collects exercise checksums into an {id: checksum} map
            let mut exercise_checksums = exercise.into_iter();
            let mut checksums = HashMap::new();
            while let Some(exercise_id) = exercise_checksums.next() {
                let exercise_id = exercise_id.parse().map_err(|err| {
                    anyhow::anyhow!("Failed to parse exercise id '{exercise_id}': {err}")
                })?;
                let checksum = exercise_checksums
                    .next()
                    .expect("the argument takes two values");
                checksums.insert(exercise_id, checksum.to_string());
            }

            let update_result = client
                .get_exercise_updates(course_id, checksums)
                .context("Failed to get exercise updates")?;
            CliOutput::finished_with_data(
                "fetched exercise updates",
                DataKind::UpdateResult(update_result),
            )
        }

        TestMyCodeCommand::GetOrganization { organization } => {
            let org = client
                .get_organization(&organization)
                .context("Failed to get organization")?;
            CliOutput::finished_with_data("fetched organization", DataKind::Organization(org))
        }

        TestMyCodeCommand::GetOrganizations => {
            let orgs = client
                .get_organizations()
                .context("Failed to get organizations")?;
            CliOutput::finished_with_data("fetched organizations", DataKind::Organizations(orgs))
        }

        TestMyCodeCommand::GetUnreadReviews { course_id } => {
            let reviews = client
                .get_unread_reviews(course_id)
                .context("Failed to get unread reviews")?;
            CliOutput::finished_with_data("fetched unread reviews", DataKind::Reviews(reviews))
        }

        TestMyCodeCommand::LoggedIn => {
            if let Some(token) = auth.token() {
                CliOutput::OutputData(Box::new(OutputData {
                    status: Status::Finished,
                    message: "currently logged in".to_string(),
                    result: OutputResult::LoggedIn,
                    data: Some(DataKind::Token(token)),
                }))
            } else {
                CliOutput::OutputData(Box::new(OutputData {
                    status: Status::Finished,
                    message: "currently not logged in".to_string(),
                    result: OutputResult::NotLoggedIn,
                    data: None,
                }))
            }
        }

        TestMyCodeCommand::Logout => {
            // Only the legacy TMC token, never the mooc credentials: `mooc
            // logout` owns those, and dropping them here would log the user out
            // of the backend that issued them.
            if let Some(credentials) = auth.take_stored_tmc() {
                credentials.remove()?;
            }
            CliOutput::OutputData(Box::new(OutputData {
                status: Status::Finished,
                message: "logged out".to_string(),
                result: OutputResult::LoggedOut,
                data: None,
            }))
        }

        TestMyCodeCommand::MarkReviewAsRead {
            course_id,
            review_id,
        } => {
            client
                .mark_review_as_read(course_id, review_id)
                .context("Failed to mark review as read")?;
            CliOutput::finished("marked review as read")
        }

        TestMyCodeCommand::Paste {
            exercise_id,
            locale,
            paste_message,
            submission_path,
        } => {
            let mut lock = Lock::dir(&submission_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let tmc_project_yml = TmcProjectYml::load_or_default(&submission_path)?;
            let locale = locale.map(|l| l.0);
            let new_submission = client
                .paste(
                    exercise_id,
                    &submission_path,
                    paste_message,
                    locale,
                    tmc_project_yml.get_submission_size_limit_mb(),
                )
                .context("Failed to get paste with comment")?;
            CliOutput::finished_with_data("sent paste", DataKind::NewSubmission(new_submission))
        }

        TestMyCodeCommand::RequestCodeReview {
            exercise_id,
            locale: Locale(locale),
            message_for_reviewer,
            submission_path,
        } => {
            let mut lock = Lock::dir(&submission_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let tmc_project_yml = TmcProjectYml::load_or_default(&submission_path)?;
            let new_submission = client
                .request_code_review(
                    exercise_id,
                    &submission_path,
                    message_for_reviewer,
                    Some(locale),
                    tmc_project_yml.get_submission_size_limit_mb(),
                )
                .context("Failed to request code review")?;
            CliOutput::finished_with_data(
                "requested code review",
                DataKind::NewSubmission(new_submission),
            )
        }

        TestMyCodeCommand::ResetExercise {
            exercise_id,
            save_old_state,
            exercise_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Write)?;
            let _guard = lock.lock()?;

            if save_old_state {
                // submit current state
                let tmc_project_yml = TmcProjectYml::load_or_default(&exercise_path)?;
                client.submit(
                    exercise_id,
                    &exercise_path,
                    tmc_project_yml.get_submission_size_limit_mb(),
                    None,
                )?;
            }
            tmc_langs::reset(client, exercise_id, &exercise_path)?;
            CliOutput::finished("reset exercise")
        }

        TestMyCodeCommand::SendFeedback {
            submission_id,
            feedback_url,
            feedback,
        } => {
            let mut feedback_answers = feedback.into_iter();
            let mut feedback = vec![];
            while let Some(feedback_id) = feedback_answers.next() {
                let question_id = feedback_id.parse().map_err(|err| {
                    anyhow::anyhow!("Failed to parse feedback id '{feedback_id}': {err}")
                })?;
                let answer = feedback_answers
                    .next()
                    .expect("validation error")
                    .to_string();
                feedback.push(FeedbackAnswer {
                    question_id,
                    answer,
                });
            }

            let response = if let Some(submission_id) = submission_id {
                client
                    .send_feedback(submission_id, feedback)
                    .context("Failed to send feedback")?
            } else if let Some(feedback_url) = feedback_url {
                let feedback_url = feedback_url.parse()?;
                client.send_feedback_to_url(feedback_url, feedback)?
            } else {
                panic!("validation error")
            };
            CliOutput::finished_with_data(
                "sent feedback",
                DataKind::SubmissionFeedbackResponse(response),
            )
        }

        TestMyCodeCommand::Submit {
            dont_block,
            locale,
            submission_path,
            exercise_id,
        } => {
            let mut lock = Lock::dir(&submission_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let locale = locale.map(|l| l.0);
            let tmc_project_yml = TmcProjectYml::load_or_default(&submission_path)?;
            let new_submission = client
                .submit(
                    exercise_id,
                    &submission_path,
                    tmc_project_yml.get_submission_size_limit_mb(),
                    locale,
                )
                .context("Failed to submit")?;

            if dont_block {
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::NewSubmission(new_submission),
                )
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url.parse()?;
                let submission_finished = client
                    .wait_for_submission_at(submission_url)
                    .context("Failed while waiting for submissions")?;
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::SubmissionFinished(submission_finished),
                )
            }
        }

        TestMyCodeCommand::UpdateExercises => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = tmc_langs::update_tmc_exercises(client, &projects_dir)?;
            CliOutput::finished_with_data(
                "downloaded or updated exercises",
                DataKind::TmcExerciseDownload(data),
            )
        }

        TestMyCodeCommand::WaitForSubmission { submission_id } => {
            let submission_finished = client
                .wait_for_submission(submission_id)
                .context("Failed while waiting for submissions")?;
            CliOutput::finished_with_data(
                "finished waiting for submission",
                DataKind::SubmissionFinished(submission_finished),
            )
        }
    };
    Ok(output)
}

/// The OAuth2 client id the mooc device/refresh flows authenticate as. Hardcoded
/// per the shared auth contract, overridable via `TMC_LANGS_MOOC_CLIENT_ID` for
/// tests and local development.
fn mooc_client_id() -> String {
    env::var("TMC_LANGS_MOOC_CLIENT_ID").unwrap_or_else(|_| mooc::DEFAULT_CLIENT_ID.to_string())
}

fn mooc_root_url() -> Result<url::Url> {
    env::var("TMC_LANGS_MOOC_ROOT_URL")
        .unwrap_or_else(|_| "https://courses.mooc.fi/".to_string())
        .parse()
        .context("Invalid Courses MOOC root url")
}

fn run_mooc(mooc: Mooc) -> Result<CliOutput> {
    let root_url = mooc_root_url()?;
    let client_id = mooc_client_id();
    let client_name = mooc.client_name.clone();

    // Auth commands are handled before initializing the client: there is no
    // token yet, so the auth-failure handling below must not apply to them.
    match &mooc.command {
        MoocCommand::Login => return run_mooc_login(&client_name, &root_url, &client_id),
        MoocCommand::LoggedIn => return mooc_logged_in(&client_name),
        MoocCommand::Logout => return mooc_logout(&client_name),
        _ => {}
    }

    let (client, credentials) =
        tmc_langs::init_mooc_client_with_credentials(root_url.clone(), &client_name, &client_id)?;
    let had_credentials = credentials.is_some();
    let auth = tmc_langs::MoocAuth::new(client_name.clone(), root_url.clone(), client_id.clone());

    match run_mooc_inner(mooc, &client, &auth) {
        Ok(output) => Ok(output),
        Err(error) => {
            // Each network call retries itself on a 401 (see
            // `MoocCredentials::call_with_refresh`), so credentials are only deleted mid-call
            // on a permanent rejection. Check that durable fact instead of the error's shape,
            // which can get rewrapped on its way here.
            if had_credentials && tmc_langs::MoocCredentials::load(&client_name)?.is_none() {
                log::error!(
                    "mooc credentials were deleted during the call, reporting invalid token"
                );
                return Err(InvalidTokenError { source: error }.into());
            }
            Err(error)
        }
    }
}

/// Logs in to the mooc backend via the OAuth2 device authorization grant.
///
/// Requests a device + user code, emits a `mooc-device-login` status update so
/// the client can show the user the verification URL *before* blocking, then
/// polls the token endpoint until the login is approved. On success the token
/// pair is saved. Cancellation is by the parent process killing this one;
/// nothing is persisted until success, so no cleanup is needed.
fn run_mooc_login(client_name: &str, root_url: &url::Url, client_id: &str) -> Result<CliOutput> {
    let device = mooc::device_authorization(root_url, client_id)?;

    // Emit the verification info before blocking on the poll loop.
    progress_reporter::start_stage::<output::MoocDeviceLogin>(
        1,
        "Waiting for device authorization".to_string(),
        Some(output::MoocDeviceLogin {
            verification_uri: device.verification_uri.clone(),
            verification_uri_complete: device.verification_uri_complete.clone(),
            user_code: device.user_code.clone(),
            expires_in: device.expires_in,
            interval: device.interval,
        }),
    );

    let token = poll_mooc_device_login(root_url, client_id, &device)?;
    tmc_langs::MoocCredentials::save(client_name, token)?;

    Ok(CliOutput::OutputData(Box::new(OutputData {
        status: Status::Finished,
        message: "logged in".to_string(),
        result: OutputResult::LoggedIn,
        data: None,
    })))
}

/// The env var overriding the device-login poll period. See
/// [`device_poll_delay`].
const MOOC_DEVICE_POLL_INTERVAL_VAR: &str = "TMC_LANGS_MOOC_DEVICE_POLL_INTERVAL_MS";

/// Decides how long to wait before the next device-login poll.
///
/// In production the base is the server-provided interval grown by 5s for every
/// `slow_down` seen so far (RFC 8628 §3.5). `TMC_LANGS_MOOC_DEVICE_POLL_INTERVAL_MS`,
/// when set, overrides the whole computation and wins regardless of the backoff:
/// it is a **test knob** that intentionally bypasses the real multi-second
/// interval (and the backoff) so tests need not sleep. It is unset in
/// production, where the server interval and slow_down backoff apply.
fn device_poll_delay(
    server_interval_secs: u64,
    slow_down_count: u64,
    env_override: Option<String>,
) -> Duration {
    let base_secs = server_interval_secs.saturating_add(5u64.saturating_mul(slow_down_count));
    let base_ms = base_secs.saturating_mul(1000);
    let ms = parse_poll_millis(MOOC_DEVICE_POLL_INTERVAL_VAR, env_override, base_ms, 10);
    Duration::from_millis(ms)
}

/// Polls the token endpoint until the device login is approved, honoring the
/// server's interval and increasing it by 5s on `slow_down` (RFC 8628 §3.5).
/// The poll period is env-tunable (`TMC_LANGS_MOOC_DEVICE_POLL_INTERVAL_MS`) so
/// tests need not sleep the real multi-second interval; the backoff decision
/// itself lives in the pure [`device_poll_delay`].
fn poll_mooc_device_login(
    root_url: &url::Url,
    client_id: &str,
    device: &mooc::DeviceAuthorizationResponse,
) -> Result<mooc::api::Token> {
    let server_interval_secs = device.interval as u64;
    // Number of `slow_down` responses so far; grows the base interval by 5s each.
    let mut slow_down_count: u64 = 0;
    let deadline = Instant::now() + Duration::from_secs(device.expires_in as u64);

    loop {
        let delay = device_poll_delay(
            server_interval_secs,
            slow_down_count,
            env::var(MOOC_DEVICE_POLL_INTERVAL_VAR).ok(),
        );
        thread::sleep(delay);

        if Instant::now() >= deadline {
            // Locally bound the wait; the CLI maps this to `not-logged-in`.
            return Err(Box::new(mooc::MoocClientError::DeviceCodeExpired).into());
        }

        match mooc::poll_device_token(root_url, client_id, &device.device_code)? {
            mooc::DeviceTokenPoll::Pending => {}
            mooc::DeviceTokenPoll::SlowDown => {
                slow_down_count = slow_down_count.saturating_add(1);
            }
            mooc::DeviceTokenPoll::Authorized(token) => return Ok(*token),
        }
    }
}

/// Reports whether mooc credentials are stored, printing the token if so —
/// mirrors the tmc `logged-in` command.
fn mooc_logged_in(client_name: &str) -> Result<CliOutput> {
    if let Some(credentials) = tmc_langs::MoocCredentials::load(client_name)? {
        Ok(CliOutput::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: "currently logged in".to_string(),
            result: OutputResult::LoggedIn,
            data: Some(DataKind::Token(credentials.token())),
        })))
    } else {
        Ok(CliOutput::OutputData(Box::new(OutputData {
            status: Status::Finished,
            message: "currently not logged in".to_string(),
            result: OutputResult::NotLoggedIn,
            data: None,
        })))
    }
}

/// Removes the stored mooc credentials — mirrors the tmc `logout` command.
fn mooc_logout(client_name: &str) -> Result<CliOutput> {
    if let Some(credentials) = tmc_langs::MoocCredentials::load(client_name)? {
        credentials.remove()?;
    }
    Ok(CliOutput::OutputData(Box::new(OutputData {
        status: Status::Finished,
        message: "logged out".to_string(),
        result: OutputResult::LoggedOut,
        data: None,
    })))
}

fn run_mooc_inner(
    mooc: Mooc,
    client: &MoocClient,
    auth: &tmc_langs::MoocAuth,
) -> Result<CliOutput> {
    let client_name = &mooc.client_name;

    let output = match mooc.command {
        // Handled in `run_mooc` before the client is initialized.
        MoocCommand::Login | MoocCommand::LoggedIn | MoocCommand::Logout => {
            unreachable!("auth commands are dispatched before client initialization")
        }
        MoocCommand::CheckExerciseUpdates => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let course = tmc_langs::check_mooc_exercise_updates(client, auth, &projects_dir)?;
            CliOutput::finished_with_data(
                "checked exercise updates",
                DataKind::MoocUpdatedExercises(course),
            )
        }
        MoocCommand::Course { course_id } => {
            let course = auth.call(client, |c| c.course(course_id))?;
            CliOutput::finished_with_data("fetched course", DataKind::MoocCourse(course))
        }
        MoocCommand::Courses => {
            let course = auth.call(client, |c| c.courses())?;
            CliOutput::finished_with_data("fetched course", DataKind::MoocCourses(course))
        }
        MoocCommand::CourseExercises { course_id } => {
            let course_exercises = auth.call(client, |c| c.course_exercises(course_id))?;
            CliOutput::finished_with_data(
                "fetched course exercises",
                DataKind::MoocExerciseSlides(course_exercises),
            )
        }
        MoocCommand::CourseProgress { course_id } => {
            let progress = auth.call(client, |c| c.course_progress(course_id))?;
            CliOutput::finished_with_data(
                "fetched course progress",
                DataKind::MoocCourseProgress(progress),
            )
        }
        MoocCommand::Exercise { exercise_id } => {
            let exercise = auth.call(client, |c| c.exercise(exercise_id))?;
            CliOutput::finished_with_data("fetched exercise", DataKind::MoocExerciseSlide(exercise))
        }
        MoocCommand::DownloadExercise {
            exercise_id,
            target,
        } => {
            let exercise = auth.call(client, |c| c.download_exercise(exercise_id))?;
            tmc_langs::extract_project(
                Cursor::new(exercise),
                &target,
                Compression::TarZstd,
                false,
                false,
            )?;
            CliOutput::finished("downloaded exercise")
        }
        MoocCommand::DownloadOrUpdateCourseExercises {
            exercise_id: exercise_ids,
            course_id,
        } => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = tmc_langs::download_or_update_mooc_course_exercises(
                client,
                auth,
                &projects_dir,
                &exercise_ids,
                course_id,
            )?;
            CliOutput::finished_with_data(
                "downloaded or updated exercises",
                DataKind::MoocExerciseDownload(data),
            )
        }
        MoocCommand::ListLocalCourseExercises { course_id } => {
            let local_exercises =
                tmc_langs::list_local_mooc_course_exercises(client_name, course_id)?;
            CliOutput::finished_with_data(
                format!("listed local exercises for {course_id}"),
                DataKind::LocalMoocExercises(local_exercises),
            )
        }
        MoocCommand::Submit {
            exercise_id,
            submission_path,
            dont_block,
        } => {
            let mut lock = Lock::dir(&submission_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let temp = file_util::named_temp_file()?;
            tmc_langs::compress_project_to(
                &submission_path,
                temp.path(),
                Compression::TarZstd,
                false,
                false,
            )?;

            let result = auth.call(client, |c| c.submit_exercise(exercise_id, temp.path()))?;
            if dont_block {
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::MoocSubmissionFinished(result),
                )
            } else {
                let status = wait_for_mooc_grading(client, auth, result.submission_id)?;
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::MoocSubmissionStatus(status),
                )
            }
        }
        MoocCommand::WaitForGrading { submission_id } => {
            let status = wait_for_mooc_grading(client, auth, submission_id)?;
            CliOutput::finished_with_data(
                "waited for grading",
                DataKind::MoocSubmissionStatus(status),
            )
        }
        MoocCommand::Paste {
            exercise_id,
            submission_path,
        } => {
            let mut lock = Lock::dir(&submission_path, LockOptions::Read)?;
            let _guard = lock.lock()?;

            let temp = file_util::named_temp_file()?;
            tmc_langs::compress_project_to(
                &submission_path,
                temp.path(),
                Compression::TarZstd,
                false,
                false,
            )?;

            // Submit non-blocking. The submit response carries a TASK-submission
            // id, but sharing takes a SLIDE-submission id, so we can't share it
            // directly. Instead we list the exercise's submissions (newest first)
            // and share the one we just created — its `id` is the slide-submission
            // id `share_submission` expects. Each request is refreshed-and-retried
            // independently, so a 401 on the (idempotent) list or share step never
            // re-runs the (non-idempotent) submit above.
            auth.call(client, |c| c.submit_exercise(exercise_id, temp.path()))?;
            let newest = auth
                .call(client, |c| c.get_exercise_submissions(exercise_id))?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no submission found for exercise {exercise_id} after submitting it"
                    )
                })?;
            let paste = auth.call(client, |c| c.share_submission(newest.id))?;
            CliOutput::finished_with_data("pasted exercise", DataKind::MoocPaste(paste))
        }
        MoocCommand::GetExerciseSubmissions { exercise_id } => {
            let submissions = auth.call(client, |c| c.get_exercise_submissions(exercise_id))?;
            CliOutput::finished_with_data(
                "fetched exercise submissions",
                DataKind::MoocSubmissions(submissions),
            )
        }
        MoocCommand::DownloadOldSubmission {
            submission_id,
            exercise_id,
            output_path,
            save_old_state,
        } => {
            let mut output_lock = Lock::dir(&output_path, LockOptions::Write)?;
            let output_guard = output_lock.lock()?;

            tmc_langs::download_mooc_old_submission(
                client,
                auth,
                exercise_id,
                &output_path,
                submission_id,
                save_old_state,
            )?;
            drop(output_guard);
            output_lock.forget();
            CliOutput::finished("extracted project")
        }
        MoocCommand::ResetExercise {
            save_old_state,
            exercise_id,
            exercise_path,
        } => {
            let mut lock = Lock::dir(&exercise_path, LockOptions::Write)?;
            let _guard = lock.lock()?;

            tmc_langs::reset_mooc_exercise(
                client,
                auth,
                exercise_id,
                &exercise_path,
                save_old_state,
            )?;
            CliOutput::finished("reset exercise")
        }
        MoocCommand::UpdateExercises => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let res = tmc_langs::update_mooc_exercises(client, auth, &projects_dir)?;
            CliOutput::finished_with_data("updated exercises", DataKind::MoocExerciseDownload(res))
        }
    };
    Ok(output)
}

/// Default grading-poll interval and the overall wait bound, both overridable via
/// env vars so tests can poll fast without sleeping real seconds. The backend
/// grades asynchronously, so a blocking submit must poll, bounded so the caller
/// is never wedged forever.
fn mooc_poll_config() -> (Duration, Duration) {
    fn millis(var: &str, default: u64, min: u64) -> Duration {
        Duration::from_millis(parse_poll_millis(var, env::var(var).ok(), default, min))
    }
    (
        // Floor the interval at a sane minimum so a bogus `0` can't busy-loop.
        millis("TMC_LANGS_MOOC_POLL_INTERVAL_MS", 2000, 10),
        millis("TMC_LANGS_MOOC_POLL_TIMEOUT_MS", 180_000, 0),
    )
}

/// Parses a poll-knob millisecond value. `None` yields `default`; a valid value
/// yields `value.max(min)` (so it can be floored); an invalid one logs a warning
/// and falls back to `default`.
fn parse_poll_millis(var: &str, raw: Option<String>, default: u64, min: u64) -> u64 {
    match raw {
        None => default,
        Some(s) => match s.parse::<u64>() {
            Ok(value) => value.max(min),
            Err(_) => {
                log::warn!("ignoring unparseable {var}={s:?}, falling back to default {default}ms");
                default
            }
        },
    }
}

/// Whether a grading status is terminal from the student's point of view.
///
/// `FullyGraded`/`Failed` are final; `PendingManual` is terminal-for-student
/// (the automated part is done, results won't change synchronously — the student
/// sees "awaiting manual grading"). `Pending`/`NotReady` and `NoGradingYet` keep
/// the loop polling.
fn mooc_grading_is_terminal(status: &mooc::ExerciseTaskSubmissionStatus) -> bool {
    use mooc::{ExerciseTaskSubmissionStatus as Status, GradingProgress as Progress};
    match status {
        Status::NoGradingYet => false,
        Status::Grading {
            grading_progress, ..
        } => matches!(
            grading_progress,
            Progress::FullyGraded | Progress::Failed | Progress::PendingManual
        ),
    }
}

/// A human-readable progress message for the current grading status.
fn mooc_grading_message(status: &mooc::ExerciseTaskSubmissionStatus) -> String {
    use mooc::{ExerciseTaskSubmissionStatus as Status, GradingProgress as Progress};
    match status {
        Status::NoGradingYet => "Grading has not started yet".to_string(),
        Status::Grading {
            grading_progress, ..
        } => match grading_progress {
            Progress::NotReady => "Grading not ready".to_string(),
            Progress::Pending => "Grading in progress".to_string(),
            Progress::PendingManual => "Awaiting manual grading".to_string(),
            Progress::FullyGraded => "Fully graded".to_string(),
            Progress::Failed => "Grading failed".to_string(),
        },
    }
}

/// Polls a submission's grading until it reaches a terminal state (see
/// [`mooc_grading_is_terminal`]) or the timeout elapses, emitting stdout progress
/// updates as the TMC submit loop does. On timeout the latest non-terminal status
/// is returned as data (not an error), so the caller can show "still grading"
/// rather than treat the wait as a failure.
///
/// Each poll goes through [`tmc_langs::MoocAuth::call`], so a 401 is
/// refreshed-and-retried transparently without resubmitting anything. Only a
/// *permanent* auth failure aborts the wait immediately; anything else is
/// treated as a transient poll error and retried until the deadline.
fn wait_for_mooc_grading(
    client: &MoocClient,
    auth: &tmc_langs::MoocAuth,
    submission_id: Uuid,
) -> Result<mooc::ExerciseTaskSubmissionStatus> {
    let (interval, timeout) = mooc_poll_config();
    progress_reporter::start_stage::<()>(1, "Waiting for grading".to_string(), None);
    let deadline = Instant::now() + timeout;
    loop {
        match auth.call(client, |c| c.get_submission_grading(submission_id)) {
            Ok(status) => {
                if mooc_grading_is_terminal(&status) {
                    progress_reporter::finish_stage::<()>(mooc_grading_message(&status), None);
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    progress_reporter::finish_stage::<()>(
                        "Grading still in progress, stopped waiting".to_string(),
                        None,
                    );
                    return Ok(status);
                }
                progress_reporter::progress_stage::<()>(mooc_grading_message(&status), None);
            }
            // The token was permanently rejected: waiting any longer cannot
            // help (there is no session left to poll with), so fail fast
            // instead of burning the rest of the poll timeout.
            Err(e) if e.is_permanent() => {
                progress_reporter::finish_stage::<()>(
                    "Grading status unavailable, stopped waiting".to_string(),
                    None,
                );
                return Err(e.into());
            }
            // A transient poll error (network blip, 5xx, or a refresh that
            // itself failed transiently) must not abort a submit that the
            // backend has already recorded: keep polling until the deadline
            // and only surface a persistent failure then.
            Err(e) => {
                if Instant::now() >= deadline {
                    progress_reporter::finish_stage::<()>(
                        "Grading status unavailable, stopped waiting".to_string(),
                        None,
                    );
                    return Err(e.into());
                }
                log::warn!("transient error while polling grading, retrying: {e}");
                progress_reporter::progress_stage::<()>(
                    "Grading status temporarily unavailable, retrying".to_string(),
                    None,
                );
            }
        }
        thread::sleep(interval);
    }
}

fn run_settings(settings: Settings) -> Result<CliOutput> {
    let client_name = &settings.client_name;
    let output = match settings.command {
        SettingsCommand::Get { setting } => {
            let value = tmc_langs::get_setting(client_name, &setting)?;
            CliOutput::finished_with_data("retrieved value", DataKind::ConfigValue(value))
        }

        SettingsCommand::List => {
            let tmc_config = tmc_langs::get_settings(client_name)?;
            CliOutput::finished_with_data("retrieved settings", DataKind::TmcConfig(tmc_config))
        }

        SettingsCommand::Migrate {
            exercise_path,
            course_slug,
            exercise_id,
            exercise_slug,
            exercise_checksum,
        } => {
            let tmc_config = TmcConfig::load(client_name)?;
            tmc_langs::migrate_exercise(
                tmc_config,
                &course_slug,
                &exercise_slug,
                exercise_id,
                &exercise_checksum,
                &exercise_path,
            )?;
            CliOutput::finished("migrated exercise")
        }

        SettingsCommand::MoveProjectsDir { dir } => {
            let tmc_config = TmcConfig::load(client_name)?;
            tmc_langs::move_projects_dir(tmc_config, dir)?;
            CliOutput::finished("moved project directory")
        }

        SettingsCommand::Reset => {
            tmc_langs::reset_settings(client_name)?;
            CliOutput::finished("reset settings")
        }

        SettingsCommand::Set { key, json, base64 } => {
            let json: Value = if base64 {
                let json = base64::engine::general_purpose::STANDARD.decode(&json)?;
                deserialize::json_from_slice(&json)?
            } else {
                deserialize::json_from_str(&json)?
            };
            tmc_langs::set_setting(client_name, &key, json)?;
            CliOutput::finished("set setting")
        }

        SettingsCommand::Unset { setting } => {
            tmc_langs::unset_setting(client_name, &setting)?;
            CliOutput::finished("unset setting")
        }
    };
    Ok(output)
}

fn write_result_to_file_as_json<T: Serialize>(
    result: &T,
    output_path: &Path,
    pretty: bool,
    secret: Option<String>,
) -> Result<()> {
    let mut output_lock =
        Lock::file(output_path, LockOptions::WriteTruncate).with_context(|| {
            format!(
                "Failed to create results JSON file at {}",
                output_path.display()
            )
        })?;
    let mut output_guard = output_lock.lock()?;

    if let Some(secret) = secret {
        let token = tmc_langs::sign_with_jwt(result, secret.as_bytes())?;
        file_util::write_to_writer(token, output_guard.get_file_mut())
            .with_context(|| format!("Failed to write result to {}", output_path.display()))?;
    } else if pretty {
        serde_json::to_writer_pretty(output_guard.get_file_mut(), result).with_context(|| {
            format!(
                "Failed to write result as JSON to {}",
                output_path.display()
            )
        })?;
    } else {
        serde_json::to_writer(output_guard.get_file_mut(), result).with_context(|| {
            format!(
                "Failed to write result as JSON to {}",
                output_path.display()
            )
        })?;
    }

    Ok(())
}

// if output_path is Some, the checkstyle results are written to that path
fn run_checkstyle_write_results(
    exercise_path: &Path,
    output_path: Option<&Path>,
    locale: Language,
) -> Result<Option<StyleValidationResult>> {
    let check_result = tmc_langs::checkstyle(exercise_path, locale).with_context(|| {
        format!(
            "Failed to check code style for project at {}",
            exercise_path.display()
        )
    })?;
    if let Some(output_path) = output_path {
        let mut output_lock =
            Lock::file(output_path, LockOptions::WriteTruncate).with_context(|| {
                format!(
                    "Failed to create code style check results file at {}",
                    output_path.display()
                )
            })?;
        let mut output_guard = output_lock.lock()?;

        serde_json::to_writer(output_guard.get_file_mut(), &check_result).with_context(|| {
            format!(
                "Failed to write code style check results as JSON to {}",
                output_path.display()
            )
        })?;
    }
    Ok(check_result)
}

#[cfg(test)]
mod test {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_display_help() {
        let cli = Cli::try_parse_from(["tmc-langs-cli", "--help"]);
        if let ParsingResult::Help(err) = map_parsing_result(cli) {
            assert!(err.to_string().contains("Usage:"));
        } else {
            panic!()
        }
    }

    #[test]
    fn parses_version() {
        let cli = Cli::try_parse_from(["tmc-langs-cli", "--version"]);
        if let ParsingResult::Version(err) = map_parsing_result(cli) {
            assert!(err.to_string().starts_with("tmc-langs-cli"));
        } else {
            panic!()
        }
    }

    #[test]
    fn parse_poll_millis_uses_default_when_unset() {
        assert_eq!(parse_poll_millis("VAR", None, 2000, 10), 2000);
    }

    #[test]
    fn parse_poll_millis_floors_to_min() {
        // A bogus `0` interval would busy-loop, so it is floored to `min`.
        assert_eq!(
            parse_poll_millis("VAR", Some("0".to_string()), 2000, 10),
            10
        );
    }

    #[test]
    fn parse_poll_millis_passes_through_valid_value() {
        assert_eq!(
            parse_poll_millis("VAR", Some("500".to_string()), 2000, 10),
            500
        );
    }

    #[test]
    fn parse_poll_millis_falls_back_on_unparseable() {
        assert_eq!(
            parse_poll_millis("VAR", Some("abc".to_string()), 2000, 10),
            2000
        );
    }

    #[test]
    fn device_poll_backoff_grows_by_5s_per_slow_down() {
        // No env override (the production case): base is the server interval, and
        // each observed `slow_down` grows the wait by 5s (RFC 8628 §3.5).
        assert_eq!(device_poll_delay(5, 0, None), Duration::from_secs(5));
        assert_eq!(device_poll_delay(5, 1, None), Duration::from_secs(10));
        assert_eq!(device_poll_delay(5, 2, None), Duration::from_secs(15));
        // A different server interval backs off from that base.
        assert_eq!(device_poll_delay(3, 2, None), Duration::from_secs(13));
    }

    #[test]
    fn device_poll_env_override_bypasses_backoff() {
        // The test knob wins regardless of the slow_down count, so the backoff is
        // intentionally bypassed when the override is set.
        assert_eq!(
            device_poll_delay(5, 0, Some("50".to_string())),
            Duration::from_millis(50)
        );
        assert_eq!(
            device_poll_delay(5, 3, Some("50".to_string())),
            Duration::from_millis(50)
        );
        // Still floored to the 10ms minimum so a bogus `0` can't busy-loop.
        assert_eq!(
            device_poll_delay(5, 3, Some("0".to_string())),
            Duration::from_millis(10)
        );
    }
}
