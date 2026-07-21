#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! CLI client for TMC.

pub mod app;
pub mod error;
pub mod output;

use self::{
    error::{DownloadsFailedError, InvalidTokenError, SandboxTestError},
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
    io::{self, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tmc_langs::{
    CommandError, Compression, Credentials, DownloadOrUpdateTmcCourseExercisesResult, Language,
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
        match cause.downcast_ref::<MoocClientError>().or_else(|| {
            cause
                .downcast_ref::<Box<MoocClientError>>()
                .map(Box::as_ref)
        }) {
            Some(MoocClientError::HttpError {
                url: _,
                status,
                error: _,
                obsolete_client,
                message_key,
            }) => {
                // `message_key` is the backend's stable identifier for a
                // controlled error; prefer it over the raw status where present.
                if *obsolete_client || message_key.as_deref() == Some("obsolete_client") {
                    return Kind::ObsoleteClient;
                }
                if message_key.as_deref() == Some("not_enrolled") {
                    return Kind::NotEnrolled;
                }
                if status.as_u16() == 403 {
                    return Kind::Forbidden;
                }
                if status.as_u16() == 401 {
                    return Kind::NotLoggedIn;
                }
            }
            Some(MoocClientError::NotAuthenticated) => {
                return Kind::NotLoggedIn;
            }
            Some(MoocClientError::ConnectionError { .. }) => {
                return Kind::ConnectionError;
            }
            _ => {}
        }

        // check for download failed error
        if let Some(DownloadsFailedError {
            downloaded: completed,
            skipped,
            failed,
        }) = cause.downcast_ref::<DownloadsFailedError>()
        {
            return Kind::FailedExerciseDownload {
                completed: completed.clone(),
                skipped: skipped.clone(),
                failed: failed.clone(),
            };
        }
    }

    Kind::Generic
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

        Command::ListLocalCourseExercises {
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
    let (mut client, mut credentials) =
        tmc_langs::init_testmycode_client_with_credentials(root_url, client_name, client_version)?;

    match run_tmc_inner(tmc, &mut client, &mut credentials) {
        Err(error) => {
            for cause in error.chain() {
                // check if the token was rejected and delete it if so
                if let Some(TestMyCodeClientError::HttpError { status, .. }) =
                    cause.downcast_ref::<TestMyCodeClientError>()
                {
                    if status.as_u16() == 401 {
                        log::error!("Received HTTP 401 error, deleting credentials");
                        if let Some(credentials) = credentials {
                            credentials.remove()?;
                        }
                        return Err(InvalidTokenError { source: error }.into());
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
    credentials: &mut Option<Credentials>,
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
            if let Some(credentials) = credentials {
                CliOutput::OutputData(Box::new(OutputData {
                    status: Status::Finished,
                    message: "currently logged in".to_string(),
                    result: OutputResult::LoggedIn,
                    data: Some(DataKind::Token(credentials.token())),
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

        TestMyCodeCommand::Login {
            base64,
            email,
            set_access_token,
            stdin,
        } => {
            // get token from argument or server
            let token = if let Some(token) = set_access_token {
                tmc_langs::login_with_token(token)
            } else if let Some(email) = email {
                // TODO: print "Please enter password" and add "quiet"  flag
                let password = if stdin {
                    let mut stdin = BufReader::new(std::io::stdin());
                    // the suggested replacement (read_password_with_config +
                    // input_reader) still opens /dev/tty and fails on piped stdin
                    #[allow(deprecated)]
                    rpassword::read_password_from_bufread(&mut stdin)
                        .context("Failed to read password")?
                } else {
                    rpassword::read_password().context("Failed to read password")?
                };
                let decoded = if base64 {
                    let bytes = base64::engine::general_purpose::STANDARD.decode(password)?;
                    String::from_utf8(bytes).context("Failed to decode password with base64")?
                } else {
                    password
                };
                tmc_langs::login_with_password(client, email, decoded)?
            } else {
                unreachable!("validation error");
            };

            // create token file
            Credentials::save(client_name, token)?;

            CliOutput::OutputData(Box::new(OutputData {
                status: Status::Finished,
                message: "logged in".to_string(),
                result: OutputResult::LoggedIn,
                data: None,
            }))
        }

        TestMyCodeCommand::Logout => {
            if let Some(credentials) = credentials.take() {
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

fn run_mooc(mooc: Mooc) -> Result<CliOutput> {
    let root_url = env::var("TMC_LANGS_MOOC_ROOT_URL")
        .unwrap_or_else(|_| "https://courses.mooc.fi/".to_string())
        .parse()
        .context("Invalid TMC root url")?;

    let (mut client, credentials) =
        tmc_langs::init_mooc_client_with_credentials(root_url, &mooc.client_name)?;
    match run_mooc_inner(mooc, &mut client) {
        Err(error) => {
            for cause in error.chain() {
                // Delete the credentials if the token was rejected. The mooc path
                // maps a 401 to `MoocClientError::NotAuthenticated` (not an
                // `HttpError` with a status) and returns it boxed, so match both
                // that and the boxed form to mirror the tmc path.
                let mooc_err = cause.downcast_ref::<MoocClientError>().or_else(|| {
                    cause
                        .downcast_ref::<Box<MoocClientError>>()
                        .map(Box::as_ref)
                });
                let rejected = match mooc_err {
                    Some(MoocClientError::NotAuthenticated) => true,
                    Some(MoocClientError::HttpError { status, .. }) => status.as_u16() == 401,
                    _ => false,
                };
                if rejected {
                    log::error!("mooc token was rejected, deleting credentials");
                    if let Some(credentials) = credentials {
                        credentials.remove()?;
                    }
                    return Err(InvalidTokenError { source: error }.into());
                }
            }
            Err(error)
        }
        output => output,
    }
}

fn run_mooc_inner(mooc: Mooc, client: &mut MoocClient) -> Result<CliOutput> {
    let client_name = &mooc.client_name;

    let output = match mooc.command {
        MoocCommand::CheckExerciseUpdates => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let course = tmc_langs::check_mooc_exercise_updates(client, &projects_dir)?;
            CliOutput::finished_with_data(
                "checked exercise updates",
                DataKind::MoocUpdatedExercises(course),
            )
        }
        MoocCommand::Course { course_id } => {
            let course = client.course(course_id)?;
            CliOutput::finished_with_data("fetched course", DataKind::MoocCourse(course))
        }
        MoocCommand::Courses => {
            let course = client.courses()?;
            CliOutput::finished_with_data("fetched course", DataKind::MoocCourses(course))
        }
        MoocCommand::CourseExercises { course_id } => {
            let course_exercises = client.course_exercises(course_id)?;
            CliOutput::finished_with_data(
                "fetched course exercises",
                DataKind::MoocExerciseSlides(course_exercises),
            )
        }
        MoocCommand::Exercise { exercise_id } => {
            let exercise = client.exercise(exercise_id)?;
            CliOutput::finished_with_data("fetched exercise", DataKind::MoocExerciseSlide(exercise))
        }
        MoocCommand::DownloadExercise {
            exercise_id,
            target,
        } => {
            let exercise = client.download_exercise(exercise_id)?;
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

            let result = client.submit_exercise(exercise_id, temp.path())?;
            if dont_block {
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::MoocSubmissionFinished(result),
                )
            } else {
                let status = wait_for_mooc_grading(client, result.submission_id)?;
                CliOutput::finished_with_data(
                    "submitted exercise",
                    DataKind::MoocSubmissionStatus(status),
                )
            }
        }
        MoocCommand::WaitForGrading { submission_id } => {
            let status = wait_for_mooc_grading(client, submission_id)?;
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
            // id `share_submission` expects.
            client.submit_exercise(exercise_id, temp.path())?;
            let newest = client
                .get_exercise_submissions(exercise_id)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no submission found for exercise {exercise_id} after submitting it"
                    )
                })?;
            let paste = client.share_submission(newest.id)?;
            CliOutput::finished_with_data("pasted exercise", DataKind::MoocPaste(paste))
        }
        MoocCommand::GetExerciseSubmissions { exercise_id } => {
            let submissions = client.get_exercise_submissions(exercise_id)?;
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
                exercise_id,
                &output_path,
                submission_id,
                save_old_state,
            )?;
            drop(output_guard);
            output_lock.forget();
            CliOutput::finished("extracted project")
        }
        MoocCommand::UpdateExercises => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let res = tmc_langs::update_mooc_exercises(client, &projects_dir)?;
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
fn wait_for_mooc_grading(
    client: &MoocClient,
    submission_id: Uuid,
) -> Result<mooc::ExerciseTaskSubmissionStatus> {
    let (interval, timeout) = mooc_poll_config();
    progress_reporter::start_stage::<()>(1, "Waiting for grading".to_string(), None);
    let deadline = Instant::now() + timeout;
    loop {
        // A transient poll error (network blip, 5xx) must not abort a submit that
        // the backend has already recorded: keep polling until the deadline and
        // only surface a persistent failure then.
        match client.get_submission_grading(submission_id) {
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
}
