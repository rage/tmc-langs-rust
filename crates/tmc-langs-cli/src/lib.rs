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
use app::{Command, Core, Settings};
use base64::Engine;
use clap::{error::ErrorKind, CommandFactory};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    env,
    fs::File,
    io::{self, BufReader, Cursor, Read},
    ops::Deref,
    path::{Path, PathBuf},
};
use tmc_langs::{
    file_util, ClientError, CommandError, Credentials, DownloadOrUpdateCourseExercisesResult,
    DownloadResult, FeedbackAnswer, Language, StyleValidationResult, TmcClient, TmcConfig,
    UpdatedExercise,
};
use tmc_langs_util::deserialize;

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
    pub output: CliOutput,
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
                output,
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

        // check for client errors
        match cause.downcast_ref::<ClientError>() {
            Some(ClientError::HttpError {
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
            Some(ClientError::NotAuthenticated) => {
                return Kind::NotLoggedIn;
            }
            Some(ClientError::ConnectionError(..)) => {
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
    let output = match cli.cmd {
        Command::Checkstyle {
            exercise_path,
            locale: Locale(locale),
            output_path,
        } => {
            file_util::lock!(exercise_path);
            let check_result =
                run_checkstyle_write_results(&exercise_path, output_path.as_deref(), locale)?;
            CliOutput::finished_with_data("ran checkstyle", check_result.map(DataKind::Validation))
        }

        Command::Clean { exercise_path } => {
            file_util::lock!(exercise_path);
            tmc_langs::clean(&exercise_path)?;
            CliOutput::finished(format!("cleaned exercise at {}", exercise_path.display()))
        }

        Command::CompressProject {
            exercise_path,
            output_path,
            compression,
            naive,
        } => {
            file_util::lock!(exercise_path);
            tmc_langs::compress_project_to(&exercise_path, &output_path, compression, naive)?;
            CliOutput::finished(format!(
                "compressed project from {} to {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::Core(core) => {
            let client_name = require_client_name(&cli.client_name)?;
            let client_version = require_client_version(&cli.client_version)?;
            run_core(client_name, client_version, core)?
        }

        Command::ExtractProject {
            archive_path,
            output_path,
            compression,
            naive,
        } => {
            let mut archive = file_util::open_file_locked(&archive_path)?;
            let mut guard = archive.write()?;

            let mut data = vec![];
            guard.read_to_end(&mut data)?;

            tmc_langs::extract_project(Cursor::new(data), &output_path, compression, true, naive)?;

            CliOutput::finished(format!(
                "extracted project from {} to {}",
                archive_path.display(),
                output_path.display()
            ))
        }

        Command::FastAvailablePoints { exercise_path } => {
            file_util::lock!(exercise_path);
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
            file_util::lock!(search_path);
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
            file_util::lock!(exercise_path);
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

        Command::ListLocalCourseExercises { course_slug } => {
            let client_name = require_client_name(&cli.client_name)?;

            let local_exercises =
                tmc_langs::list_local_course_exercises(client_name, &course_slug)?;

            CliOutput::finished_with_data(
                format!("listed local exercises for {course_slug}"),
                DataKind::LocalExercises(local_exercises),
            )
        }

        Command::PrepareSolution {
            exercise_path,
            output_path,
        } => {
            file_util::lock!(exercise_path);
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
            file_util::lock!(exercise_path);
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
            file_util::lock!(exercise_path);

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

        Command::Settings(settings) => {
            let client_name = require_client_name(&cli.client_name)?;
            run_settings(client_name, settings)?
        }

        Command::ScanExercise {
            exercise_path,
            output_path,
        } => {
            file_util::lock!(exercise_path);

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
    };
    Ok(output)
}

fn run_core(client_name: &str, client_version: &str, core: Core) -> Result<CliOutput> {
    let root_url = env::var("TMC_LANGS_ROOT_URL")
        .unwrap_or_else(|_| "https://tmc.mooc.fi/".to_string())
        .parse()
        .context("Invalid TMC root url")?;
    let (client, mut credentials) =
        tmc_langs::init_tmc_client_with_credentials(root_url, client_name, client_version)?;

    match run_core_inner(client_name, core, client, &mut credentials) {
        Err(error) => {
            for cause in error.chain() {
                // check if the token was rejected and delete it if so
                if let Some(ClientError::HttpError { status, .. }) =
                    cause.downcast_ref::<ClientError>()
                {
                    if status.as_u16() == 401 {
                        log::error!("Received HTTP 401 error, deleting credentials");
                        if let Some(credentials) = credentials {
                            credentials.remove()?;
                        }
                        return Err(InvalidTokenError { source: error }.into());
                    } else {
                        log::warn!("401 without credentials");
                    }
                }
            }
            Err(error)
        }
        output => output,
    }
}

fn run_core_inner(
    client_name: &str,
    core: Core,
    mut client: TmcClient,
    credentials: &mut Option<Credentials>,
) -> Result<CliOutput> {
    let output = match core {
        Core::CheckExerciseUpdates => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let updated_exercises = tmc_langs::check_exercise_updates(&client, &projects_dir)
                .context("Failed to check exercise updates")?
                .into_iter()
                .map(|id| UpdatedExercise { id })
                .collect::<Vec<_>>();

            CliOutput::finished_with_data(
                "updated exercises",
                DataKind::UpdatedExercises(updated_exercises),
            )
        }

        Core::DownloadModelSolution {
            exercise_id,
            target,
        } => {
            client
                .download_model_solution(exercise_id, &target)
                .context("Failed to download model solution")?;
            CliOutput::finished("downloaded model solution")
        }

        Core::DownloadOldSubmission {
            submission_id,
            save_old_state,
            exercise_id,
            output_path,
        } => {
            tmc_langs::download_old_submission(
                &client,
                exercise_id,
                &output_path,
                submission_id,
                save_old_state,
            )?;
            CliOutput::finished("extracted project")
        }

        Core::DownloadOrUpdateCourseExercises {
            download_template,
            exercise_id: exercise_ids,
        } => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = match tmc_langs::download_or_update_course_exercises(
                &client,
                &projects_dir,
                &exercise_ids,
                download_template,
            )? {
                DownloadResult::Success {
                    downloaded,
                    skipped,
                } => DownloadOrUpdateCourseExercisesResult {
                    downloaded,
                    skipped,
                    failed: None,
                },
                DownloadResult::Failure {
                    downloaded,
                    skipped,
                    failed,
                } => DownloadOrUpdateCourseExercisesResult {
                    downloaded,
                    skipped,
                    failed: Some(failed),
                },
            };
            CliOutput::finished_with_data(
                "downloaded or updated exercises",
                DataKind::ExerciseDownload(data),
            )
        }

        Core::GetCourseData { course_id } => {
            let data = tmc_langs::get_course_data(&client, course_id)
                .context("Failed to get course data")?;
            CliOutput::finished_with_data(
                "fetched course data",
                DataKind::CombinedCourseData(Box::new(data)),
            )
        }

        Core::GetCourseDetails { course_id } => {
            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;
            CliOutput::finished_with_data(
                "fetched course details",
                DataKind::CourseDetails(details),
            )
        }

        Core::GetCourseExercises { course_id } => {
            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data(
                "fetched course exercises",
                DataKind::CourseExercises(exercises),
            )
        }

        Core::GetCourseSettings { course_id } => {
            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data("fetched course settings", DataKind::CourseData(settings))
        }

        Core::GetCourses { organization } => {
            let courses = client
                .list_courses(&organization)
                .context("Failed to get courses")?;
            CliOutput::finished_with_data("fetched courses", DataKind::Courses(courses))
        }

        Core::GetExerciseDetails { exercise_id } => {
            let course = client
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;
            CliOutput::finished_with_data(
                "fetched exercise details",
                DataKind::ExerciseDetails(course),
            )
        }

        Core::GetExerciseSubmissions { exercise_id } => {
            let submissions = client
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;
            CliOutput::finished_with_data(
                "fetched exercise submissions",
                DataKind::Submissions(submissions),
            )
        }

        Core::GetExerciseUpdates {
            course_id,
            exercise,
        } => {
            // collects exercise checksums into an {id: checksum} map
            let mut exercise_checksums = exercise.into_iter();
            let mut checksums = HashMap::new();
            while let Some(exercise_id) = exercise_checksums.next() {
                let exercise_id = into_u32(&exercise_id)?;
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

        Core::GetOrganization { organization } => {
            let org = client
                .get_organization(&organization)
                .context("Failed to get organization")?;
            CliOutput::finished_with_data("fetched organization", DataKind::Organization(org))
        }

        Core::GetOrganizations => {
            let orgs = client
                .get_organizations()
                .context("Failed to get organizations")?;
            CliOutput::finished_with_data("fetched organizations", DataKind::Organizations(orgs))
        }

        Core::GetUnreadReviews { course_id } => {
            let reviews = client
                .get_unread_reviews(course_id)
                .context("Failed to get unread reviews")?;
            CliOutput::finished_with_data("fetched unread reviews", DataKind::Reviews(reviews))
        }

        Core::LoggedIn => {
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

        Core::Login {
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
                tmc_langs::login_with_password(&mut client, client_name, email, decoded)?
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

        Core::Logout => {
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

        Core::MarkReviewAsRead {
            course_id,
            review_id,
        } => {
            client
                .mark_review_as_read(course_id, review_id)
                .context("Failed to mark review as read")?;
            CliOutput::finished("marked review as read")
        }

        Core::Paste {
            exercise_id,
            locale,
            paste_message,
            submission_path,
        } => {
            file_util::lock!(submission_path);
            let locale = locale.map(|l| l.0);
            let new_submission = client
                .paste(exercise_id, &submission_path, paste_message, locale)
                .context("Failed to get paste with comment")?;
            CliOutput::finished_with_data("sent paste", DataKind::NewSubmission(new_submission))
        }

        Core::RequestCodeReview {
            exercise_id,
            locale: Locale(locale),
            message_for_reviewer,
            submission_path,
        } => {
            file_util::lock!(submission_path);
            let new_submission = client
                .request_code_review(
                    exercise_id,
                    &submission_path,
                    message_for_reviewer,
                    Some(locale),
                )
                .context("Failed to request code review")?;
            CliOutput::finished_with_data(
                "requested code review",
                DataKind::NewSubmission(new_submission),
            )
        }

        Core::ResetExercise {
            exercise_id,
            save_old_state,
            exercise_path,
        } => {
            file_util::lock!(exercise_path);
            if save_old_state {
                // submit current state
                client.submit(exercise_id, &exercise_path, None)?;
            }
            tmc_langs::reset(&client, exercise_id, &exercise_path)?;
            CliOutput::finished("reset exercise")
        }

        Core::SendFeedback {
            submission_id,
            feedback_url,
            feedback,
        } => {
            let mut feedback_answers = feedback.into_iter();
            let mut feedback = vec![];
            while let Some(feedback_id) = feedback_answers.next() {
                let question_id = into_u32(&feedback_id)?;
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

        Core::Submit {
            dont_block,
            locale,
            submission_path,
            exercise_id,
        } => {
            file_util::lock!(submission_path);
            let locale = locale.map(|l| l.0);
            let new_submission = client
                .submit(exercise_id, &submission_path, locale)
                .context("Failed to submit")?;

            if dont_block {
                CliOutput::finished_with_data(
                    "submit exercise",
                    DataKind::NewSubmission(new_submission),
                )
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url.parse()?;
                let submission_finished = client
                    .wait_for_submission_at(submission_url)
                    .context("Failed while waiting for submissions")?;
                CliOutput::finished_with_data(
                    "submit exercise",
                    DataKind::SubmissionFinished(submission_finished),
                )
            }
        }

        Core::UpdateExercises => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = tmc_langs::update_exercises(&client, &projects_dir)?;
            CliOutput::finished_with_data(
                "downloaded or updated exercises",
                DataKind::ExerciseDownload(data),
            )
        }

        Core::WaitForSubmission { submission_id } => {
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

fn run_settings(client_name: &str, settings: Settings) -> Result<CliOutput> {
    let output = match settings {
        Settings::Get { setting } => {
            let value = tmc_langs::get_setting(client_name, &setting)?;
            CliOutput::finished_with_data("retrieved value", DataKind::ConfigValue(value))
        }

        Settings::List => {
            let tmc_config = tmc_langs::get_settings(client_name)?;
            CliOutput::finished_with_data("retrieved settings", DataKind::TmcConfig(tmc_config))
        }

        Settings::Migrate {
            exercise_path,
            course_slug,
            exercise_id,
            exercise_slug,
            exercise_checksum,
        } => {
            let config_path = TmcConfig::get_location(client_name)?;
            let tmc_config = TmcConfig::load(client_name, &config_path)?;
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

        Settings::MoveProjectsDir { dir } => {
            let config_path = TmcConfig::get_location(client_name)?;
            let tmc_config = TmcConfig::load(client_name, &config_path)?;
            tmc_langs::move_projects_dir(tmc_config, &config_path, dir)?;
            CliOutput::finished("moved project directory")
        }

        Settings::Reset => {
            tmc_langs::reset_settings(client_name)?;
            CliOutput::finished("reset settings")
        }

        Settings::Set { key, json, base64 } => {
            let json: Value = if base64 {
                let json = base64::engine::general_purpose::STANDARD.decode(&json)?;
                deserialize::json_from_slice(&json)?
            } else {
                deserialize::json_from_str(&json)?
            };
            tmc_langs::set_setting(client_name, &key, json)?;
            CliOutput::finished("set setting")
        }

        Settings::Unset { setting } => {
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
    let mut output_file = file_util::create_file_locked(output_path).with_context(|| {
        format!(
            "Failed to create results JSON file at {}",
            output_path.display()
        )
    })?;
    let guard = output_file.write()?;

    if let Some(secret) = secret {
        let token = tmc_langs::sign_with_jwt(result, secret.as_bytes())?;
        file_util::write_to_writer(token, guard.deref())
            .with_context(|| format!("Failed to write result to {}", output_path.display()))?;
    } else if pretty {
        serde_json::to_writer_pretty(guard.deref(), result).with_context(|| {
            format!(
                "Failed to write result as JSON to {}",
                output_path.display()
            )
        })?;
    } else {
        serde_json::to_writer(guard.deref(), result).with_context(|| {
            format!(
                "Failed to write result as JSON to {}",
                output_path.display()
            )
        })?;
    }

    Ok(())
}

fn into_u32(arg: &str) -> Result<u32> {
    arg.parse::<u32>()
        .with_context(|| format!("Failed to convert argument to a non-negative integer: {arg}"))
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
        let output_file = File::create(output_path).with_context(|| {
            format!(
                "Failed to create code style check results file at {}",
                output_path.display()
            )
        })?;
        serde_json::to_writer(output_file, &check_result).with_context(|| {
            format!(
                "Failed to write code style check results as JSON to {}",
                output_path.display()
            )
        })?;
    }
    Ok(check_result)
}

fn require_client_name(client_name: &Option<String>) -> Result<&str> {
    if let Some(client_name) = client_name.as_ref() {
        Ok(client_name)
    } else {
        anyhow::bail!(
            "The following required argument was not provided: --client-name <client-name>"
        );
    }
}

fn require_client_version(client_version: &Option<String>) -> Result<&str> {
    if let Some(client_version) = client_version.as_ref() {
        Ok(client_version)
    } else {
        anyhow::bail!(
            "The following required argument was not provided: --client-version <client-version>"
        );
    }
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
}
