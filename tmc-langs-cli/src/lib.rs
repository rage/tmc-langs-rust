#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! CLI client for TMC

mod app;
mod error;
mod output;

use self::error::{DownloadsFailedError, InvalidTokenError, SandboxTestError};
use self::output::{
    Data, Kind, Output, OutputData, OutputResult, Status, StatusUpdateData, UpdatedExercise,
};
use crate::app::{Locale, Opt};
use anyhow::{Context, Result};
use app::{Command, Core, OutputFormatWrapper, Settings};
use clap::{Error, ErrorKind};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use structopt::StructOpt;
use tmc_langs::{
    file_util, notification_reporter, ClientError, ClientUpdateData, CommandError, Credentials,
    DownloadOrUpdateCourseExercisesResult, DownloadResult, FeedbackAnswer, Language,
    StyleValidationResult, TmcClient, TmcConfig,
};
use tmc_langs_util::progress_reporter;

// wraps the run_inner function that actually does the work and handles any panics that occur
// any langs library should never panic by itself, but other libraries used may in some rare circumstances
pub fn run() {
    // run the inner function and catch any panics
    match std::panic::catch_unwind(run_inner) {
        Ok(res) => {
            // no panic, output was printed properly
            match res {
                Ok(_) => {
                    // inner returned Ok, exit with 0
                    quit::with_code(0);
                }
                Err(_) => {
                    // inner returned Err, exit with 1
                    quit::with_code(1);
                }
            }
        }
        Err(err) => {
            // panicked, likely before any output was printed
            // currently only prints a message if the panic is called with str or String; this should be good enough
            let error_message = if let Some(string) = err.downcast_ref::<&str>() {
                format!("Process panicked unexpectedly with message: {}", string)
            } else if let Some(string) = err.downcast_ref::<String>() {
                format!("Process panicked unexpectedly with message: {}", string)
            } else {
                "Process panicked unexpectedly without an error message".to_string()
            };
            let output = Output::OutputData(OutputData {
                status: Status::Crashed,
                message: error_message,
                result: OutputResult::Error,
                data: None,
            });
            print_output(&output, false).expect("this should never fail");

            quit::with_code(1);
        }
    }
}

// sets up warning and progress reporting and calls run_app and does error handling for its result
// returns Ok if we should exit with code 0, Err if we should exit with 1
fn run_inner() -> Result<(), ()> {
    let matches = Opt::from_args();
    let pretty = matches.pretty;

    notification_reporter::init(Box::new(move |warning| {
        let warning_output = Output::Notification(warning);
        if let Err(err) = print_output(&warning_output, pretty) {
            log::error!("printing warning failed: {}", err);
        }
    }));

    progress_reporter::subscribe::<(), _>(move |update| {
        let output = Output::StatusUpdate(StatusUpdateData::None(update));
        let _r = print_output(&output, pretty);
    });

    progress_reporter::subscribe::<ClientUpdateData, _>(move |update| {
        let output = Output::StatusUpdate(StatusUpdateData::ClientUpdateData(update));
        let _r = print_output(&output, pretty);
    });

    if let Err(e) = run_app(matches) {
        // error handling
        let causes: Vec<String> = e.chain().map(|e| format!("Caused by: {}", e)).collect();
        let message = error_message_special_casing(&e);
        let kind = solve_error_kind(&e);
        let sandbox_path = check_sandbox_err(&e);
        let error_output = Output::OutputData(OutputData {
            status: Status::Finished,
            message,
            result: OutputResult::Error,
            data: Some(Data::Error {
                kind,
                trace: causes,
            }),
        });
        print_output_with_file(&error_output, pretty, sandbox_path)
            .expect("failed to print output");
        Err(())
    } else {
        Ok(())
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
            Some(ClientError::NotLoggedIn) => {
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

fn run_app(matches: Opt) -> Result<()> {
    let output = match matches.cmd {
        Command::Checkstyle {
            exercise_path,
            locale: Locale(locale),
            output_path,
        } => {
            file_util::lock!(exercise_path);
            let check_result =
                run_checkstyle_write_results(&exercise_path, output_path.as_deref(), locale)?;
            Output::finished_with_data("ran checkstyle", check_result.map(Data::Validation))
        }

        Command::Clean { exercise_path } => {
            file_util::lock!(exercise_path);
            tmc_langs::clean(&exercise_path)?;
            Output::finished(format!("cleaned exercise at {}", exercise_path.display()))
        }

        Command::CompressProject {
            exercise_path,
            output_path,
        } => {
            file_util::lock!(exercise_path);
            tmc_langs::compress_project_to(&exercise_path, &output_path)?;
            Output::finished(format!(
                "compressed project from {} to {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::Core(core) => {
            let client_name = require_client_name(&matches.client_name)?;
            let client_version = require_client_version(&matches.client_version)?;
            run_core(client_name, client_version, core)?
        }

        Command::ExtractProject {
            archive_path,
            output_path,
        } => {
            let mut archive = file_util::open_file_lock(&archive_path)?;
            let mut guard = archive.lock()?;

            let mut data = vec![];
            guard.read_to_end(&mut data)?;

            tmc_langs::extract_project(Cursor::new(data), &output_path, true)?;

            Output::finished(format!(
                "extracted project from {} to {}",
                archive_path.display(),
                output_path.display()
            ))
        }

        Command::FastAvailablePoints { exercise_path } => {
            file_util::lock!(exercise_path);
            let points = tmc_langs::get_available_points(&exercise_path)?;
            Output::finished_with_data(
                format!("found {} available points", points.len()),
                Data::AvailablePoints(points),
            )
        }

        Command::FindExercises {
            exercise_path,
            output_path,
        } => {
            file_util::lock!(exercise_path);
            let exercises =
                tmc_langs::find_exercise_directories(&exercise_path).with_context(|| {
                    format!(
                        "Failed to find exercise directories in {}",
                        exercise_path.display(),
                    )
                })?;
            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&exercises, &output_path, matches.pretty, None)?;
            }
            Output::finished_with_data(
                format!("found exercises at {}", exercise_path.display()),
                Data::Exercises(exercises),
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
                write_result_to_file_as_json(&config, &output_path, matches.pretty, None)?;
            }
            Output::finished_with_data(
                format!(
                    "created exercise packaging config from {}",
                    exercise_path.display(),
                ),
                Data::ExercisePackagingConfiguration(config),
            )
        }

        Command::ListLocalCourseExercises { course_slug } => {
            let client_name = require_client_name(&matches.client_name)?;

            let local_exercises =
                tmc_langs::list_local_course_exercises(client_name, &course_slug)?;

            Output::finished_with_data(
                format!("listed local exercises for {}", course_slug),
                Data::LocalExercises(local_exercises),
            )
        }

        Command::PrepareSolutions {
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
            Output::finished(format!(
                "prepared solutions for {} at {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::PrepareStubs {
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
            Output::finished(format!(
                "prepared stubs for {} at {}",
                exercise_path.display(),
                output_path.display()
            ))
        }

        Command::PrepareSubmission {
            clone_path,
            output_format: OutputFormatWrapper(output_format),
            output_path,
            stub_zip_path,
            submission_path,
            tmc_param,
            top_level_dir_name,
        } => {
            // will contain for each key all the values with that key in a list
            let mut tmc_params_grouped = HashMap::new();
            for value in &tmc_param {
                let params: Vec<_> = value.split('=').collect();
                if params.len() != 2 {
                    Error::with_description(
                        "tmc-param values should contain a single '=' as a delimiter.",
                        ErrorKind::ValueValidation,
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

            tmc_langs::prepare_submission(
                &submission_path,
                &output_path,
                top_level_dir_name,
                tmc_params,
                &clone_path,
                stub_zip_path.as_deref(),
                output_format,
            )?;
            Output::finished(format!(
                "prepared submission for {} at {}",
                submission_path.display(),
                output_path.display()
            ))
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
            .with_context(|| format!("Failed to refresh course {}", course_name))?;
            Output::finished_with_data(
                format!("refreshed course {}", course_name),
                Data::RefreshResult(refresh_result),
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
                write_result_to_file_as_json(&test_result, &output_path, matches.pretty, secret)?;
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

            Output::finished_with_data(
                format!("ran tests for {}", exercise_path.display()),
                Data::TestResult(test_result),
            )
        }

        Command::Settings(settings) => {
            let client_name = require_client_name(&matches.client_name)?;
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
                format!(
                    "Exercise path's file name '{:?}' was not valid UTF8",
                    exercise_name
                )
            })?;

            let scan_result = tmc_langs::scan_exercise(&exercise_path, exercise_name.to_string())
                .with_context(|| {
                format!("Failed to scan exercise at {}", exercise_path.display())
            })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&scan_result, &output_path, matches.pretty, None)?;
            }

            Output::finished_with_data(
                format!("scanned exercise at {}", exercise_path.display()),
                Data::ExerciseDesc(scan_result),
            )
        }
    };
    print_output(&output, matches.pretty)?;
    Ok(())
}

fn run_core(client_name: &str, client_version: &str, core: Core) -> Result<Output> {
    let root_url =
        env::var("TMC_LANGS_ROOT_URL").unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
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
) -> Result<Output> {
    let output = match core {
        Core::CheckExerciseUpdates => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let updated_exercises = tmc_langs::check_exercise_updates(&client, &projects_dir)
                .context("Failed to check exercise updates")?
                .into_iter()
                .map(|id| UpdatedExercise { id })
                .collect();

            Output::finished_with_data(
                "updated exercises",
                Data::UpdatedExercises(updated_exercises),
            )
        }

        Core::DownloadModelSolution {
            solution_download_url,
            target,
        } => {
            client
                .download_model_solution(solution_download_url, &target)
                .context("Failed to download model solution")?;
            Output::finished("downloaded model solution")
        }

        Core::DownloadOldSubmission {
            save_old_state: _,
            exercise_id,
            output_path,
            submission_id,
            submission_url,
        } => {
            tmc_langs::download_old_submission(
                &client,
                exercise_id,
                &output_path,
                submission_id,
                submission_url,
            )?;
            Output::finished("extracted project")
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
            Output::finished_with_data(
                "downloaded or updated exercises",
                Data::ExerciseDownload(data),
            )
        }

        Core::GetCourseData { course_id } => {
            let data = tmc_langs::get_course_data(&client, course_id)
                .context("Failed to get course data")?;
            Output::finished_with_data(
                "fetched course data",
                Data::CombinedCourseData(Box::new(data)),
            )
        }

        Core::GetCourseDetails { course_id } => {
            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;
            Output::finished_with_data("fetched course details", Data::CourseDetails(details))
        }

        Core::GetCourseExercises { course_id } => {
            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;
            Output::finished_with_data("fetched course exercises", Data::CourseExercises(exercises))
        }

        Core::GetCourseSettings { course_id } => {
            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;
            Output::finished_with_data("fetched course settings", Data::CourseData(settings))
        }

        Core::GetCourses { organization } => {
            let courses = client
                .list_courses(&organization)
                .context("Failed to get courses")?;
            Output::finished_with_data("fetched courses", Data::Courses(courses))
        }

        Core::GetExerciseDetails { exercise_id } => {
            let course = client
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;
            Output::finished_with_data("fetched exercise details", Data::ExerciseDetails(course))
        }

        Core::GetExerciseSubmissions { exercise_id } => {
            let submissions = client
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;
            Output::finished_with_data(
                "fetched exercise submissions",
                Data::Submissions(submissions),
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
            Output::finished_with_data(
                "fetched exercise updates",
                Data::UpdateResult(update_result),
            )
        }

        Core::GetOrganization { organization } => {
            let org = client
                .get_organization(&organization)
                .context("Failed to get organization")?;
            Output::finished_with_data("fetched organization", Data::Organization(org))
        }

        Core::GetOrganizations => {
            let orgs = client
                .get_organizations()
                .context("Failed to get organizations")?;
            Output::finished_with_data("fetched organizations", Data::Organizations(orgs))
        }

        Core::GetUnreadReviews { reviews_url } => {
            let reviews = client
                .get_unread_reviews(reviews_url)
                .context("Failed to get unread reviews")?;
            Output::finished_with_data("fetched unread reviews", Data::Reviews(reviews))
        }

        Core::LoggedIn => {
            if let Some(credentials) = credentials {
                Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: "currently logged in".to_string(),
                    result: OutputResult::LoggedIn,
                    data: Some(Data::Token(credentials.token())),
                })
            } else {
                Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: "currently not logged in".to_string(),
                    result: OutputResult::NotLoggedIn,
                    data: None,
                })
            }
        }

        Core::Login {
            base64,
            email,
            set_access_token,
        } => {
            // get token from argument or server
            let token = if let Some(token) = set_access_token {
                tmc_langs::login_with_token(token)
            } else if let Some(email) = email {
                // TODO: print "Please enter password" and add "quiet"  flag
                let password = rpassword::read_password().context("Failed to read password")?;
                let decoded = if base64 {
                    let bytes = base64::decode(password)?;
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

            Output::OutputData(OutputData {
                status: Status::Finished,
                message: "logged in".to_string(),
                result: OutputResult::LoggedIn,
                data: None,
            })
        }

        Core::Logout => {
            if let Some(credentials) = credentials.take() {
                credentials.remove()?;
            }
            Output::OutputData(OutputData {
                status: Status::Finished,
                message: "logged out".to_string(),
                result: OutputResult::LoggedOut,
                data: None,
            })
        }

        Core::MarkReviewAsRead { review_update_url } => {
            client
                .mark_review_as_read(review_update_url.to_string())
                .context("Failed to mark review as read")?;
            Output::finished("marked review as read")
        }

        Core::Paste {
            locale,
            paste_message,
            submission_path,
            submission_url,
        } => {
            file_util::lock!(submission_path);
            let locale = locale.map(|l| l.0);
            let new_submission = client
                .paste(submission_url, &submission_path, paste_message, locale)
                .context("Failed to get paste with comment")?;
            Output::finished_with_data("sent paste", Data::NewSubmission(new_submission))
        }

        Core::RequestCodeReview {
            locale: Locale(locale),
            message_for_reviewer,
            submission_path,
            submission_url,
        } => {
            file_util::lock!(submission_path);
            let new_submission = client
                .request_code_review(
                    submission_url,
                    &submission_path,
                    message_for_reviewer,
                    Some(locale),
                )
                .context("Failed to request code review")?;
            Output::finished_with_data("requested code review", Data::NewSubmission(new_submission))
        }

        Core::ResetExercise {
            save_old_state,
            exercise_id,
            exercise_path,
            submission_url,
        } => {
            file_util::lock!(exercise_path);
            if save_old_state {
                // submit current state
                client.submit(
                    submission_url.expect("validation error"),
                    &exercise_path,
                    None,
                )?;
            }
            tmc_langs::reset(&client, exercise_id, &exercise_path)?;
            Output::finished("reset exercise")
        }

        Core::SendFeedback {
            feedback,
            feedback_url,
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
            let response = client
                .send_feedback(feedback_url, feedback)
                .context("Failed to send feedback")?;
            Output::finished_with_data("sent feedback", Data::SubmissionFeedbackResponse(response))
        }

        Core::Submit {
            dont_block,
            locale,
            submission_path,
            submission_url,
        } => {
            file_util::lock!(submission_path);
            let locale = locale.map(|l| l.0);
            let new_submission = client
                .submit(submission_url, &submission_path, locale)
                .context("Failed to submit")?;

            if dont_block {
                Output::finished_with_data("submit exercise", Data::NewSubmission(new_submission))
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url;
                let submission_finished = client
                    .wait_for_submission(&submission_url)
                    .context("Failed while waiting for submissions")?;
                Output::finished_with_data(
                    "submit exercise",
                    Data::SubmissionFinished(submission_finished),
                )
            }
        }

        Core::UpdateExercises => {
            let projects_dir = tmc_langs::get_projects_dir(client_name)?;
            let data = tmc_langs::update_exercises(&client, &projects_dir)?;
            Output::finished_with_data(
                "downloaded or updated exercises",
                Data::ExerciseDownload(data),
            )
        }

        Core::WaitForSubmission { submission_url } => {
            let submission_finished = client
                .wait_for_submission(submission_url.as_ref())
                .context("Failed while waiting for submissions")?;
            Output::finished_with_data(
                "finished waiting for submission",
                Data::SubmissionFinished(submission_finished),
            )
        }
    };
    Ok(output)
}

fn run_settings(client_name: &str, settings: Settings) -> Result<Output> {
    let output = match settings {
        Settings::Get { setting } => {
            let value = tmc_langs::get_setting(client_name, &setting)?;
            Output::finished_with_data("retrieved value", Data::ConfigValue(value))
        }

        Settings::List => {
            let tmc_config = tmc_langs::get_settings(client_name)?;
            Output::finished_with_data("retrieved settings", Data::TmcConfig(tmc_config))
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
            Output::finished("migrated exercise")
        }

        Settings::MoveProjectsDir { dir } => {
            let config_path = TmcConfig::get_location(client_name)?;
            let tmc_config = TmcConfig::load(client_name, &config_path)?;
            tmc_langs::move_projects_dir(tmc_config, &config_path, dir)?;
            Output::finished("moved project directory")
        }

        Settings::Set { key, json } => {
            tmc_langs::set_setting(client_name, &key, &json.to_string())?;
            Output::finished("set setting")
        }

        Settings::Reset => {
            tmc_langs::reset_settings(client_name)?;
            Output::finished("reset settings")
        }

        Settings::Unset { setting } => {
            tmc_langs::unset_setting(client_name, &setting)?;
            Output::finished("unset setting")
        }
    };
    Ok(output)
}

fn print_output(output: &Output, pretty: bool) -> Result<PrintToken> {
    print_output_with_file(output, pretty, None)
}

#[allow(clippy::clippy::print_stdout)] // this is the only function that should output to stdout/stderr across tmc-langs
fn print_output_with_file(
    output: &Output,
    pretty: bool,
    path: Option<PathBuf>,
) -> Result<PrintToken> {
    let result = if pretty {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .with_context(|| format!("Failed to convert {:?} to JSON", output))?;
    println!("{}", result);

    if let Some(path) = path {
        let mut file = File::create(&path)
            .with_context(|| format!("Failed to open file at {}", path.display()))?;
        file.write_all(result.as_bytes())
            .with_context(|| format!("Failed to write result to {}", path.display()))?;
    }
    Ok(PrintToken)
}

fn write_result_to_file_as_json<T: Serialize>(
    result: &T,
    output_path: &Path,
    pretty: bool,
    secret: Option<String>,
) -> Result<()> {
    let mut output_file = file_util::create_file_lock(output_path).with_context(|| {
        format!(
            "Failed to create results JSON file at {}",
            output_path.display()
        )
    })?;
    let guard = output_file.lock()?;

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
    arg.parse::<u32>().with_context(|| {
        format!(
            "Failed to convert argument to a non-negative integer: {}",
            arg,
        )
    })
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

struct PrintToken;
