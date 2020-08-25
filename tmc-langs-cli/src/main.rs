//! CLI client for TMC

mod app;
mod output;

use output::{CombinedCourseData, DownloadTarget, ErrorData, Kind, Output, OutputResult, Status};

use anyhow::{Context, Result};
use clap::{ArgMatches, Error, ErrorKind};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tmc_langs_core::oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
use tmc_langs_core::{CoreError, FeedbackAnswer, StatusType, TmcCore, Token};
use tmc_langs_framework::{domain::ValidationResult, error::CommandError};
use tmc_langs_util::{
    task_executor::{self, TmcParams},
    Language, OutputFormat,
};
use url::Url;

#[quit::main]
fn main() {
    env_logger::init();

    if let Err(e) = run() {
        // error handling
        let causes: Vec<String> = e.chain().map(|e| format!("Caused by: {}", e)).collect();
        let message = error_message_special_casing(&e);
        let kind = solve_error_kind(&e);
        let error_output = Output {
            status: Status::Finished,
            message: Some(message),
            result: OutputResult::Error,
            data: Some(ErrorData {
                kind,
                trace: causes,
            }),
            percent_done: 1.0,
        };
        if let Err(err) = print_output(&error_output) {
            // the above function shouldn't fail ever, but in theory some data could
            // have a flawed Serialize implementation, so better safe than sorry
            let output = Output::<()> {
                status: Status::Crashed,
                message: Some(err.to_string()),
                result: OutputResult::Error,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output).expect("this should never fail");
        }
        quit::with_code(1);
    }
}

/// Goes through the error chain and checks for special error types that should be indicated by the Kind.
fn solve_error_kind(e: &anyhow::Error) -> Kind {
    for cause in e.chain() {
        // check for authorization error
        if let Some(CoreError::HttpError(_, status_code, _)) = cause.downcast_ref::<CoreError>() {
            if status_code.as_u16() == 403 {
                return Kind::AuthorizationError;
            }
        }
        // check for connection error
        if let Some(CoreError::ConnectionError(..)) = cause.downcast_ref::<CoreError>() {
            return Kind::ConnectionError;
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

fn run() -> Result<()> {
    let matches = app::create_app().get_matches();

    // enforces that each branch must return a PrintToken as proof of having printed the output
    let _printed: PrintToken = match matches.subcommand() {
        ("checkstyle", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let check_result = run_checkstyle_write_results(exercise_path, output_path, locale)?;

            let output = Output {
                status: Status::Finished,
                message: Some("ran checkstyle".to_string()),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: check_result,
            };
            print_output(&output)?
        }
        ("clean", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            task_executor::clean(exercise_path).with_context(|| {
                format!("Failed to clean exercise at {}", exercise_path.display(),)
            })?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!("cleaned exercise at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("compress-project", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let data = task_executor::compress_project(exercise_path).with_context(|| {
                format!("Failed to compress project at {}", exercise_path.display())
            })?;

            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory {}", parent.display()))?;
            }
            fs::write(output_path, &data).with_context(|| {
                format!(
                    "Failed to write compressed project to {}",
                    output_path.display()
                )
            })?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!(
                    "compressed project from {} to {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("core", Some(matches)) => run_core(matches)?,
        ("extract-project", Some(matches)) => {
            let archive_path = matches.value_of("archive-path").unwrap();
            let archive_path = Path::new(archive_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            task_executor::extract_project(archive_path, output_path, true).with_context(|| {
                format!("Failed to extract project at {}", output_path.display())
            })?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!(
                    "extracted project from {} to {}",
                    archive_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("find-exercises", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let exercises = task_executor::find_exercise_directories(exercise_path);

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&exercises, output_path)?;
            }

            let output = Output {
                status: Status::Finished,
                message: Some(format!("found exercises at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(exercises),
            };
            print_output(&output)?
        }
        ("get-exercise-packaging-configuration", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let config = task_executor::get_exercise_packaging_configuration(exercise_path)
                .with_context(|| {
                    format!(
                        "Failed to get exercise packaging configuration for exercise at {}",
                        exercise_path.display(),
                    )
                })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&config, output_path)?;
            }

            let output = Output {
                status: Status::Finished,
                message: Some(format!(
                    "created exercise packaging config from {}",
                    exercise_path.display(),
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(config),
            };
            print_output(&output)?
        }
        ("prepare-solutions", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            task_executor::prepare_solutions(&[exercise_path.to_path_buf()], output_path)
                .with_context(|| {
                    format!(
                        "Failed to prepare solutions for exercise at {}",
                        exercise_path.display(),
                    )
                })?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!(
                    "prepared solutions for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("prepare-stubs", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let exercises = task_executor::find_exercise_directories(exercise_path);

            task_executor::prepare_stubs(exercises, exercise_path, output_path).with_context(
                || {
                    format!(
                        "Failed to prepare stubs for exercise at {}",
                        exercise_path.display(),
                    )
                },
            )?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!(
                    "prepared stubs for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("prepare-submission", Some(matches)) => {
            let output_format = match matches.value_of("output-format") {
                Some("tar") => OutputFormat::Tar,
                Some("zip") => OutputFormat::Zip,
                Some("zstd") => OutputFormat::TarZstd,
                _ => unreachable!("validation error"),
            };

            let clone_path = matches.value_of("clone-path").unwrap();
            let clone_path = Path::new(clone_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let stub_zip_path = matches.value_of("stub-zip-path");
            let stub_zip_path = stub_zip_path.map(Path::new);

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let tmc_params_values = matches.values_of("tmc-param").unwrap_or_default();
            // will contain for each key all the values with that key in a list
            let mut tmc_params_grouped = HashMap::new();
            for value in tmc_params_values {
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
            let mut tmc_params = TmcParams::new();
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

            let top_level_dir_name = matches.value_of("top-level-dir-name");
            let top_level_dir_name = top_level_dir_name.map(str::to_string);

            task_executor::prepare_submission(
                submission_path,
                output_path,
                top_level_dir_name,
                tmc_params,
                clone_path,
                stub_zip_path,
                output_format,
            )?;

            let output = Output::<()> {
                status: Status::Finished,
                message: Some(format!(
                    "prepared submission for {} at {}",
                    submission_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("run-tests", Some(matches)) => {
            let checkstyle_output_path = matches.value_of("checkstyle-output-path");
            let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale");

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let test_result = task_executor::run_tests(exercise_path).with_context(|| {
                format!(
                    "Failed to run tests for exercise at {}",
                    exercise_path.display()
                )
            })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&test_result, output_path)?;
            }

            // todo: checkstyle results in stdout?
            if let Some(checkstyle_output_path) = checkstyle_output_path {
                let locale = into_locale(locale.unwrap())?;

                run_checkstyle_write_results(exercise_path, Some(checkstyle_output_path), locale)?;
            }

            let output = Output {
                status: Status::Finished,
                message: Some(format!("ran tests for {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(test_result),
            };
            print_output(&output)?
        }
        ("scan-exercise", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

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

            let scan_result =
                task_executor::scan_exercise(exercise_path, exercise_name.to_string())
                    .with_context(|| {
                        format!("Failed to scan exercise at {}", exercise_path.display())
                    })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&scan_result, output_path)?;
            }

            let output = Output {
                status: Status::Finished,
                message: Some(format!("scanned exercise at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(scan_result),
            };
            print_output(&output)?
        }
        _ => unreachable!("missing subcommand arm"),
    };
    Ok(())
}

fn run_core(matches: &ArgMatches) -> Result<PrintToken> {
    let client_name = matches.value_of("client-name").unwrap();

    let client_version = matches.value_of("client-version").unwrap();

    let root_url =
        env::var("TMC_LANGS_ROOT_URL").unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
    let mut core = TmcCore::new_in_config(
        root_url,
        client_name.to_string(),
        client_version.to_string(),
    )
    .context("Failed to create TmcCore")?;
    // set progress report to print the updates to stdout as JSON
    core.set_progress_report(|update| {
        // convert to output
        let data = match &update.status_type {
            StatusType::DownloadingExercise { id, path }
            | StatusType::DownloadedExercise { id, path } => Some(DownloadTarget {
                id: *id,
                path: path.clone(),
            }),
            _ => None,
        };

        let output = Output {
            status: Status::InProgress,
            message: Some(update.message.to_string()),
            result: update.status_type.into(),
            percent_done: update.percent_done,
            data,
        };
        print_output(&output)?;
        Ok(())
    });

    // set token if a credentials.json is found for the client name
    let tmc_dir = format!("tmc-{}", client_name);
    let config_dir = match env::var("TMC_LANGS_CONFIG_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::config_dir().context("Failed to find config directory")?,
    };
    let credentials_path = config_dir.join(tmc_dir).join("credentials.json");
    if let Ok(file) = File::open(&credentials_path) {
        match serde_json::from_reader(file) {
            Ok(token) => core.set_token(token),
            Err(e) => {
                log::error!(
                    "Failed to deserialize credentials.json due to \"{}\", deleting",
                    e
                );
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove malformed credentials.json file {}",
                        credentials_path.display()
                    )
                })?;
            }
        }
    };

    // proof of having printed the output
    let printed: PrintToken = match matches.subcommand() {
        ("download-model-solution", Some(matches)) => {
            let solution_download_url = matches.value_of("solution-download-url").unwrap();
            let solution_download_url = into_url(solution_download_url)?;

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            core.download_model_solution(solution_download_url, target)
                .context("Failed to download model solution")?;

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("download-old-submission", Some(matches)) => {
            let save_old_state = matches.is_present("save-old-state");

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let submission_id = matches.value_of("submission-id").unwrap();
            let submission_id = into_usize(submission_id)?;

            let submission_url = matches.value_of("submission-url");

            // increment steps for reset
            core.increment_progress_steps();
            if save_old_state {
                // submit old exercise
                let submission_url = into_url(submission_url.unwrap())?;
                // increment steps for submit
                core.increment_progress_steps();
                core.submit(submission_url, output_path, None)?;
                log::debug!("finished submission");
            }

            // reset old exercise
            core.reset(exercise_id, output_path)?;
            log::debug!("reset exercise");

            // dl submission
            let temp_zip = NamedTempFile::new().context("Failed to create a temporary archive")?;
            core.download_old_submission(submission_id, temp_zip.path())?;
            log::debug!("downloaded old submission to {}", temp_zip.path().display());

            // extract submission
            task_executor::extract_student_files(temp_zip.path(), output_path)?;
            log::debug!("extracted project");

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("download-or-update-exercises", Some(matches)) => {
            let mut exercise_args = matches.values_of("exercise").unwrap();

            // collect exercise into (id, path) pairs
            let mut exercises = vec![];
            while let Some(exercise_id) = exercise_args.next() {
                let exercise_id = into_usize(exercise_id)?;
                let exercise_path = exercise_args.next().unwrap(); // safe unwrap because each --exercise takes 2 arguments
                let exercise_path = Path::new(exercise_path);
                exercises.push((exercise_id, exercise_path));
            }

            core.download_or_update_exercises(exercises)
                .context("Failed to download exercises")?;

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("get-course-data", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let details = core
                .get_course_details(course_id)
                .context("Failed to get course details")?;
            let exercises = core
                .get_course_exercises(course_id)
                .context("Failed to get course")?;
            let settings = core.get_course(course_id).context("Failed to get course")?;
            let data = CombinedCourseData {
                details,
                exercises,
                settings,
            };

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(data),
            };
            print_output(&output)?
        }
        ("get-course-details", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let details = core
                .get_course_details(course_id)
                .context("Failed to get course details")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(details),
            };
            print_output(&output)?
        }
        ("get-course-exercises", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let exercises = core
                .get_course_exercises(course_id)
                .context("Failed to get course")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(exercises),
            };
            print_output(&output)?
        }
        ("get-course-settings", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let settings = core.get_course(course_id).context("Failed to get course")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(settings),
            };
            print_output(&output)?
        }
        ("get-courses", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let courses = core
                .list_courses(organization_slug)
                .context("Failed to get courses")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(courses),
            };
            print_output(&output)?
        }
        ("get-exercise-details", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let course = core
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(course),
            };
            print_output(&output)?
        }
        ("get-exercise-submissions", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let submissions = core
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(submissions),
            };
            print_output(&output)?
        }
        ("get-exercise-updates", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            // collects exercise checksums into an {id: checksum} map
            let mut checksums = HashMap::new();
            let mut exercise_checksums = matches.values_of("exercise").unwrap();
            while let Some(exercise_id) = exercise_checksums.next() {
                let exercise_id = into_usize(exercise_id)?;
                let checksum = exercise_checksums.next().unwrap(); // safe unwrap due to exercise taking two values
                checksums.insert(exercise_id, checksum.to_string());
            }

            let update_result = core
                .get_exercise_updates(course_id, checksums)
                .context("Failed to get exercise updates")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(update_result),
            };
            print_output(&output)?
        }
        ("get-organization", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let org = core
                .get_organization(organization_slug)
                .context("Failed to get organization")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(org),
            };
            print_output(&output)?
        }
        ("get-organizations", Some(_matches)) => {
            let orgs = core
                .get_organizations()
                .context("Failed to get organizations")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(orgs),
            };
            print_output(&output)?
        }
        ("get-unread-reviews", Some(matches)) => {
            let reviews_url = matches.value_of("reviews-url").unwrap();
            let reviews_url = into_url(reviews_url)?;

            let reviews = core
                .get_unread_reviews(reviews_url)
                .context("Failed to get unread reviews")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: Some(reviews),
            };
            print_output(&output)?
        }
        ("logged-in", Some(_matches)) => {
            if credentials_path.exists() {
                let credentials = File::open(&credentials_path).with_context(|| {
                    format!(
                        "Failed to open credentials file at {}",
                        credentials_path.display()
                    )
                })?;
                let token: Token = serde_json::from_reader(credentials).with_context(|| {
                    format!(
                        "Failed to deserialize access token from {}",
                        credentials_path.display()
                    )
                })?;
                let output = Output {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::LoggedIn,
                    percent_done: 1.0,
                    data: Some(token),
                };
                print_output(&output)?
            } else {
                let output = Output::<()> {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::NotLoggedIn,
                    percent_done: 1.0,
                    data: None,
                };
                print_output(&output)?
            }
        }
        ("login", Some(matches)) => {
            let base64 = matches.is_present("base64");

            let email = matches.value_of("email");
            let set_access_token = matches.value_of("set-access-token");

            // get token from argument or server
            let token = if let Some(token) = set_access_token {
                let mut token_response = StandardTokenResponse::new(
                    AccessToken::new(token.to_string()),
                    BasicTokenType::Bearer,
                    EmptyExtraTokenFields {},
                );
                token_response.set_scopes(Some(vec![Scope::new("public".to_string())]));
                token_response
            } else if let Some(email) = email {
                // TODO: print "Please enter password" and add "quiet"  flag
                let password = rpassword::read_password().context("Failed to read password")?;
                let decoded = if base64 {
                    let bytes = base64::decode(password).context("Password was invalid base64")?;
                    String::from_utf8(bytes)
                        .context("Base64 password decoded into invalid UTF-8")?
                } else {
                    password
                };
                core.authenticate(client_name, email.to_string(), decoded)
                    .context("Failed to authenticate with TMC")?
            } else {
                unreachable!("validation error");
            };

            // create token file
            if let Some(p) = credentials_path.parent() {
                fs::create_dir_all(p)
                    .with_context(|| format!("Failed to create directory {}", p.display()))?;
            }
            let credentials_file = File::create(&credentials_path).with_context(|| {
                format!("Failed to create file at {}", credentials_path.display())
            })?;

            // write token
            if let Err(e) = serde_json::to_writer(credentials_file, &token) {
                // failed to write token, removing credentials file
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove empty credentials file after failing to write {}",
                        credentials_path.display()
                    )
                })?;
                Err(e).with_context(|| {
                    format!(
                        "Failed to write credentials to {}",
                        credentials_path.display()
                    )
                })?;
            }

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedIn,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("logout", Some(_matches)) => {
            if credentials_path.exists() {
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove credentials at {}",
                        credentials_path.display()
                    )
                })?;
            }

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("mark-review-as-read", Some(matches)) => {
            let review_update_url = matches.value_of("reiew-update-url").unwrap();

            core.mark_review_as_read(review_update_url.to_string())
                .context("Failed to mark review as read")?;

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("paste", Some(matches)) => {
            let locale = matches.value_of("locale");
            let locale = if let Some(locale) = locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let paste_message = matches.value_of("paste-message");

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let new_submission = core
                .paste(
                    submission_url,
                    submission_path,
                    paste_message.map(str::to_string),
                    locale,
                )
                .context("Failed to get paste with comment")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(new_submission),
            };
            print_output(&output)?
        }
        ("request-code-review", Some(matches)) => {
            let locale = matches.value_of("locale");
            let locale = if let Some(locale) = locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let message_for_reviewer = matches.value_of("message-for-reviewer").unwrap();

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let new_submission = core
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .context("Failed to request code review")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(new_submission),
            };
            print_output(&output)?
        }
        ("reset-exercise", Some(matches)) => {
            let save_old_state = matches.is_present("save-old-state");

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let submission_url = matches.value_of("submission-url");

            if save_old_state {
                // submit current state
                let submission_url = into_url(submission_url.unwrap())?;
                core.increment_progress_steps();
                core.submit(submission_url, exercise_path, None)?;
            }

            // reset exercise
            core.reset(exercise_id, exercise_path)?;

            let output = Output::<()> {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?
        }
        ("run-checkstyle", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let validation_result = core
                .run_checkstyle(exercise_path, locale)
                .context("Failed to run checkstyle")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(validation_result),
            };
            print_output(&output)?
        }
        ("run-tests", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let run_result = core
                .run_tests(exercise_path)
                .context("Failed to run tests")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(run_result),
            };
            print_output(&output)?
        }
        ("send-feedback", Some(matches)) => {
            // collect feedback values into a list
            let mut feedback_answers = matches.values_of("feedback").unwrap();
            let mut feedback = vec![];
            while let Some(feedback_id) = feedback_answers.next() {
                let question_id = into_usize(feedback_id)?;
                let answer = feedback_answers.next().unwrap().to_string(); // safe unwrap because --feedback always takes 2 values
                feedback.push(FeedbackAnswer {
                    question_id,
                    answer,
                });
            }

            let feedback_url = matches.value_of("feedback-url").unwrap();
            let feedback_url = into_url(feedback_url)?;

            let response = core
                .send_feedback(feedback_url, feedback)
                .context("Failed to send feedback")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(response),
            };
            print_output(&output)?
        }
        ("submit", Some(matches)) => {
            let dont_block = matches.is_present("dont-block");

            let locale = matches.value_of("locale");
            let locale = if let Some(locale) = locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            if !dont_block {
                core.increment_progress_steps();
            }
            let new_submission = core
                .submit(submission_url, submission_path, locale)
                .context("Failed to submit")?;

            if dont_block {
                let output = Output {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::SentData,
                    percent_done: 1.0,
                    data: Some(new_submission),
                };

                print_output(&output)?
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url;
                let submission_finished = core
                    .wait_for_submission(&submission_url)
                    .context("Failed while waiting for submissions")?;

                let output = Output {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::RetrievedData,
                    percent_done: 1.0,
                    data: Some(submission_finished),
                };
                print_output(&output)?
            }
        }
        ("wait-for-submission", Some(matches)) => {
            let submission_url = matches.value_of("submission-url").unwrap();

            let submission_finished = core
                .wait_for_submission(submission_url)
                .context("Failed while waiting for submissions")?;
            let submission_finished = serde_json::to_string(&submission_finished)
                .context("Failed to serialize submission results")?;

            let output = Output {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(submission_finished),
            };
            print_output(&output)?
        }
        _ => unreachable!(),
    };

    Ok(printed)
}

fn print_output<T: Serialize + Debug>(output: &Output<T>) -> Result<PrintToken> {
    let result = serde_json::to_string(&output)
        .with_context(|| format!("Failed to convert {:?} to JSON", output))?;
    println!("{}", result);
    Ok(PrintToken)
}

fn write_result_to_file_as_json<T: Serialize>(result: &T, output_path: &Path) -> Result<()> {
    let output_file = File::create(output_path).with_context(|| {
        format!(
            "Failed to create results JSON file at {}",
            output_path.display()
        )
    })?;

    serde_json::to_writer(output_file, result).with_context(|| {
        format!(
            "Failed to write result as JSON to {}",
            output_path.display()
        )
    })?;

    Ok(())
}

fn into_usize(arg: &str) -> Result<usize> {
    usize::from_str_radix(arg, 10).with_context(|| {
        format!(
            "Failed to convert argument to a non-negative integer: {}",
            arg,
        )
    })
}

fn into_locale(arg: &str) -> Result<Language> {
    Language::from_locale(arg)
        .or_else(|| Language::from_639_1(arg))
        .or_else(|| Language::from_639_3(arg))
        .with_context(|| format!("Invalid locale: {}", arg))
}

fn into_url(arg: &str) -> Result<Url> {
    Url::parse(arg).with_context(|| format!("Failed to parse url {}", arg))
}

// if output_path is Some, the checkstyle results are written to that path
fn run_checkstyle_write_results(
    exercise_path: &Path,
    output_path: Option<&Path>,
    locale: Language,
) -> Result<Option<ValidationResult>> {
    let check_result =
        task_executor::run_check_code_style(exercise_path, locale).with_context(|| {
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

struct PrintToken;
