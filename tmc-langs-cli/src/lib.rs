//! CLI client for TMC

mod app;
mod config;
mod error;
mod output;

use self::config::ProjectsConfig;
use self::config::{CourseConfig, Credentials, Exercise, TmcConfig};
use self::error::{DownloadsFailedError, InvalidTokenError, SandboxTestError};
use self::output::{
    CombinedCourseData, Data, DownloadOrUpdateCourseExercise,
    DownloadOrUpdateCourseExercisesResult, Kind, Output, OutputData, OutputResult, Status,
    StatusUpdateData, UpdatedExercise,
};
use anyhow::{Context, Result};
use clap::{ArgMatches, Error, ErrorKind};
use config::ConfigValue;
use file_util::open_file_lock;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::error::Error as StdError;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
};
use std::{env, io::Cursor};
use tempfile::NamedTempFile;
use tmc_langs::{
    file_util::{self, FileLockGuard},
    warning_reporter, CommandError, StyleValidationResult,
};
use tmc_langs::{
    oauth2::{
        basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
    },
    ClientUpdateData, Language,
};
use tmc_langs::{ClientError, FeedbackAnswer, TmcClient, Token};
use tmc_langs_util::progress_reporter;
use toml::{map::Map as TomlMap, Value as TomlValue};
use url::Url;
use walkdir::WalkDir;

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
    let matches = app::create_app().get_matches();
    let pretty = matches.is_present("pretty");

    warning_reporter::init(Box::new(move |warning| {
        let warning_output = Output::Warning(warning);
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

    if let Err(e) = run_app(matches, pretty) {
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

fn run_app(matches: ArgMatches, pretty: bool) -> Result<()> {
    // enforces that each branch must return a PrintToken as proof of having printed the output
    let _printed: PrintToken = match matches.subcommand() {
        ("checkstyle", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            file_util::lock!(exercise_path);

            let check_result = run_checkstyle_write_results(exercise_path, output_path, locale)?;

            let output =
                Output::finished_with_data("ran checkstyle", check_result.map(Data::Validation));
            print_output(&output, pretty)?
        }
        ("clean", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            file_util::lock!(exercise_path);

            tmc_langs::clean(exercise_path)?;

            let output = Output::finished_with_data(
                format!("cleaned exercise at {}", exercise_path.display()),
                None,
            );
            print_output(&output, pretty)?
        }
        ("compress-project", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            file_util::lock!(exercise_path);

            tmc_langs::compress_project_to(exercise_path, output_path)?;

            let output = Output::finished_with_data(
                format!(
                    "compressed project from {} to {}",
                    exercise_path.display(),
                    output_path.display()
                ),
                None,
            );
            print_output(&output, pretty)?
        }
        ("core", Some(matches)) => {
            let client_name = matches.value_of("client-name").unwrap();

            let client_version = matches.value_of("client-version").unwrap();

            let root_url = env::var("TMC_LANGS_ROOT_URL")
                .unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
            let mut client = TmcClient::new_in_config(
                root_url,
                client_name.to_string(),
                client_version.to_string(),
            )
            .context("Failed to create TmcClient")?;

            // set token from the credentials file if one exists
            let mut credentials = Credentials::load(client_name)?;
            if let Some(credentials) = &credentials {
                client.set_token(credentials.token())?;
            }

            match run_core(client, client_name, &mut credentials, matches, pretty) {
                Ok(token) => token,
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
                    return Err(error);
                }
            }
        }
        ("disk-space", Some(matches)) => {
            let path = matches.value_of("path").unwrap();
            let path = Path::new(path);

            let free = tmc_langs::free_disk_space_megabytes(path)?;

            let output = Output::finished_with_data(
                format!(
                    "calculated free disk space for partition containing {}",
                    path.display()
                ),
                Data::FreeDiskSpace(free),
            );
            print_output(&output, pretty)?
        }
        ("extract-project", Some(matches)) => {
            let archive_path = matches.value_of("archive-path").unwrap();
            let archive_path = Path::new(archive_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let mut archive = open_file_lock(archive_path)?;
            let mut guard = archive.lock()?;

            let mut data = vec![];
            guard.read_to_end(&mut data)?;

            tmc_langs::extract_project(Cursor::new(data), output_path, true)?;

            let output = Output::finished_with_data(
                format!(
                    "extracted project from {} to {}",
                    archive_path.display(),
                    output_path.display()
                ),
                None,
            );
            print_output(&output, pretty)?
        }
        ("fast-available-points", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            file_util::lock!(exercise_path);

            let points = tmc_langs::get_available_points(exercise_path)?;

            let output = Output::finished_with_data(
                format!("found {} available points", points.len()),
                Data::AvailablePoints(points),
            );
            print_output(&output, pretty)?
        }
        ("find-exercises", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            file_util::lock!(exercise_path);

            let exercises =
                tmc_langs::find_exercise_directories(exercise_path).with_context(|| {
                    format!(
                        "Failed to find exercise directories in {}",
                        exercise_path.display(),
                    )
                })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&exercises, output_path, pretty)?;
            }

            let output = Output::finished_with_data(
                format!("found exercises at {}", exercise_path.display()),
                Data::Exercises(exercises),
            );
            print_output(&output, pretty)?
        }
        ("get-exercise-packaging-configuration", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            file_util::lock!(exercise_path);

            let config = tmc_langs::get_exercise_packaging_configuration(exercise_path)
                .with_context(|| {
                    format!(
                        "Failed to get exercise packaging configuration for exercise at {}",
                        exercise_path.display(),
                    )
                })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&config, output_path, pretty)?;
            }

            let output = Output::finished_with_data(
                format!(
                    "created exercise packaging config from {}",
                    exercise_path.display(),
                ),
                Data::ExercisePackagingConfiguration(config),
            );
            print_output(&output, pretty)?
        }
        ("list-local-course-exercises", Some(matches)) => {
            let client_name = matches.value_of("client-name").unwrap();

            let course_slug = matches.value_of("course-slug").unwrap();

            let local_exercises = tmc_langs::list_local_course_exercises(client_name, course_slug)?;

            let output = Output::finished_with_data(
                format!("listed local exercises for {}", course_slug),
                Data::LocalExercises(local_exercises),
            );
            print_output(&output, pretty)?
        }
        ("prepare-solutions", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            file_util::lock!(exercise_path);

            tmc_langs::prepare_solution(exercise_path, output_path).with_context(|| {
                format!(
                    "Failed to prepare solutions for exercise at {}",
                    exercise_path.display(),
                )
            })?;

            let output = Output::finished_with_data(
                format!(
                    "prepared solutions for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                ),
                None,
            );
            print_output(&output, pretty)?
        }
        ("prepare-stubs", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            file_util::lock!(exercise_path);

            tmc_langs::prepare_stub(exercise_path, output_path).with_context(|| {
                format!(
                    "Failed to prepare stubs for exercise at {}",
                    exercise_path.display(),
                )
            })?;

            let output = Output::finished_with_data(
                format!(
                    "prepared stubs for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                ),
                None,
            );
            print_output(&output, pretty)?
        }
        ("prepare-submission", Some(matches)) => {
            let clone_path = matches.value_of("clone-path").unwrap();
            let clone_path = Path::new(clone_path);

            let output_format = match matches.value_of("output-format") {
                Some("tar") => tmc_langs::data::OutputFormat::Tar,
                Some("zip") => tmc_langs::data::OutputFormat::Zip,
                Some("zstd") => tmc_langs::data::OutputFormat::TarZstd,
                _ => unreachable!("validation error"),
            };

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
            let mut tmc_params = tmc_langs::data::TmcParams::new();
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

            tmc_langs::prepare_submission(
                submission_path,
                output_path,
                top_level_dir_name,
                tmc_params,
                clone_path,
                stub_zip_path,
                output_format,
            )?;

            let output = Output::finished_with_data(
                format!(
                    "prepared submission for {} at {}",
                    submission_path.display(),
                    output_path.display()
                ),
                None,
            );
            print_output(&output, pretty)?
        }
        ("refresh-course", Some(matches)) => {
            let cache_path = matches.value_of("cache-path").unwrap();
            let cache_root = matches.value_of("cache-root").unwrap();
            let course_name = matches.value_of("course-name").unwrap();
            let git_branch = matches.value_of("git-branch").unwrap();
            let source_url = matches.value_of("source-url").unwrap();

            let refresh_result = tmc_langs::refresh_course(
                course_name.to_string(),
                PathBuf::from(cache_path),
                source_url.to_string(),
                git_branch.to_string(),
                PathBuf::from(cache_root),
            )
            .with_context(|| format!("Failed to refresh course {}", course_name))?;

            let output = Output::finished_with_data(
                format!("refreshed course {}", course_name),
                Data::RefreshResult(refresh_result),
            );
            print_output(&output, pretty)?
        }
        ("run-tests", Some(matches)) => {
            let checkstyle_output_path = matches.value_of("checkstyle-output-path");
            let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale");

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            file_util::lock!(exercise_path);

            let test_result = tmc_langs::run_tests(exercise_path).with_context(|| {
                format!(
                    "Failed to run tests for exercise at {}",
                    exercise_path.display()
                )
            });

            let test_result = if env::var("TMC_SANDBOX").is_ok() {
                // in sandbox, wrap error to signal we want to write the output into a file
                test_result.map_err(|e| SandboxTestError {
                    path: output_path.map(Path::to_path_buf),
                    source: e,
                })?
            } else {
                // not in sandbox, just unwrap
                test_result?
            };

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&test_result, output_path, pretty)?;
            }

            // todo: checkstyle results in stdout?
            if let Some(checkstyle_output_path) = checkstyle_output_path {
                let locale = into_locale(locale.unwrap())?;

                run_checkstyle_write_results(exercise_path, Some(checkstyle_output_path), locale)?;
            }

            let output = Output::finished_with_data(
                format!("ran tests for {}", exercise_path.display()),
                Data::TestResult(test_result),
            );
            print_output(&output, pretty)?
        }
        ("settings", Some(matches)) => run_settings(matches, pretty)?,
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

            file_util::lock!(exercise_path);

            let scan_result = tmc_langs::scan_exercise(exercise_path, exercise_name.to_string())
                .with_context(|| {
                    format!("Failed to scan exercise at {}", exercise_path.display())
                })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&scan_result, output_path, pretty)?;
            }

            let output = Output::finished_with_data(
                format!("scanned exercise at {}", exercise_path.display()),
                Data::ExerciseDesc(scan_result),
            );
            print_output(&output, pretty)?
        }
        _ => unreachable!("missing subcommand arm"),
    };
    Ok(())
}

fn run_core(
    mut client: TmcClient,
    client_name: &str,
    credentials: &mut Option<Credentials>,
    matches: &ArgMatches,
    pretty: bool,
) -> Result<PrintToken> {
    // proof of having printed the output
    let printed: PrintToken = match matches.subcommand() {
        ("check-exercise-updates", Some(_)) => {
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
                    .get_exercises_details(exercise_ids)?
                    .into_iter()
                    .map(|e| (e.id, e))
                    .collect::<HashMap<_, _>>();
                for local_exercise in local_exercises {
                    let server_exercise =
                        server_exercises.get(&local_exercise.id).with_context(|| {
                            format!(
                                "Server did not return details for local exercise with id {}",
                                local_exercise.id
                            )
                        })?;
                    if server_exercise.checksum != local_exercise.checksum {
                        // server has an updated exercise
                        updated_exercises.push(UpdatedExercise {
                            id: local_exercise.id,
                        });
                    }
                }
            }

            let output = Output::finished_with_data(
                "updated exercises",
                Data::UpdatedExercises(updated_exercises),
            );
            print_output(&output, pretty)?
        }
        ("download-model-solution", Some(matches)) => {
            let solution_download_url = matches.value_of("solution-download-url").unwrap();
            let solution_download_url = into_url(solution_download_url)?;

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            client
                .download_model_solution(solution_download_url, target)
                .context("Failed to download model solution")?;

            let output = Output::finished_with_data("downloaded model solution", None);
            print_output(&output, pretty)?
        }
        ("download-old-submission", Some(matches)) => {
            let save_old_state = matches.is_present("save-old-state");

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = PathBuf::from(output_path);

            let submission_id = matches.value_of("submission-id").unwrap();
            let submission_id = into_usize(submission_id)?;

            let submission_url = matches.value_of("submission-url");

            if save_old_state {
                // submit old exercise
                let submission_url = into_url(submission_url.unwrap())?;
                client.submit(submission_url, &output_path, None)?;
                log::debug!("finished submission");
            }

            // reset old exercise
            client.reset(exercise_id, output_path.clone())?;
            log::debug!("reset exercise");

            // dl submission
            let temp_zip = NamedTempFile::new().context("Failed to create a temporary archive")?;
            client.download_old_submission(submission_id, temp_zip.path())?;
            log::debug!("downloaded old submission to {}", temp_zip.path().display());

            // extract submission
            tmc_langs::extract_student_files(temp_zip, &output_path)?;
            log::debug!("extracted project");

            let output = Output::finished_with_data("extracted project", None);
            print_output(&output, pretty)?
        }
        ("download-or-update-course-exercises", Some(matches)) => {
            // todo: bit of a mess, refactor
            let exercise_ids = matches.values_of("exercise-id").unwrap();

            // collect exercise into (id, path) pairs
            let exercises = exercise_ids
                .into_iter()
                .map(into_usize)
                .collect::<Result<_>>()?;
            let exercises_details = client.get_exercises_details(exercises)?;

            let config_path = TmcConfig::get_location(client_name)?;
            let projects_dir = TmcConfig::load(client_name, &config_path)?.projects_dir;
            let mut projects_config = ProjectsConfig::load(&projects_dir)?;

            // separate downloads into ones that don't need to be downloaded and ones that do
            let mut to_be_downloaded = HashMap::new();
            let mut to_be_skipped = vec![];
            for exercise_detail in exercises_details {
                let target = ProjectsConfig::get_exercise_download_target(
                    &projects_dir,
                    &exercise_detail.course_name,
                    &exercise_detail.exercise_name,
                );

                // check if the checksum is different from what's already on disk
                if let Some(course_config) =
                    projects_config.courses.get(&exercise_detail.course_name)
                {
                    if let Some(exercise) =
                        course_config.exercises.get(&exercise_detail.exercise_name)
                    {
                        if exercise_detail.checksum == exercise.checksum {
                            // skip this exercise
                            log::info!(
                                "Skipping exercise {} ({} in {}) due to identical checksum",
                                exercise_detail.id,
                                exercise_detail.course_name,
                                exercise_detail.exercise_name
                            );
                            to_be_skipped.push(DownloadOrUpdateCourseExercise {
                                course_slug: exercise_detail.course_name,
                                exercise_slug: exercise_detail.exercise_name,
                                path: target,
                            });
                            continue;
                        }
                    }
                }
                // not skipped, should be downloaded
                // also store id and checksum to be used later
                to_be_downloaded.insert(
                    exercise_detail.id,
                    (
                        DownloadOrUpdateCourseExercise {
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
                    anyhow::bail!(error)
                }
            };

            /*
            let entry = course_data.entry(exercise_detail.course_name);
            let course_exercises = entry.or_default();
            course_exercises.push((
                exercise_detail.exercise_name,
                exercise_detail.checksum,
                exercise_detail.id,
            ));

            exercises_and_paths.push((exercise_detail.id, target));
            */

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
                        let mut error = &err as &dyn StdError;
                        let mut chain = vec![error.to_string()];
                        while let Some(source) = error.source() {
                            chain.push(source.to_string());
                            error = source;
                        }
                        (ex, chain)
                    })
                    .collect();
                anyhow::bail!(DownloadsFailedError {
                    downloaded: completed,
                    skipped: to_be_skipped,
                    failed,
                })
            }

            let data = DownloadOrUpdateCourseExercisesResult {
                downloaded: completed,
                skipped: to_be_skipped,
            };
            let output = Output::finished_with_data(
                "downloaded or updated exercises",
                Data::ExerciseDownload(data),
            );
            print_output(&output, pretty)?
        }
        ("download-or-update-exercises", Some(matches)) => {
            let mut exercise_args = matches.values_of("exercise").unwrap();

            // collect exercise into (id, path) pairs
            let mut exercises = vec![];
            while let Some(exercise_id) = exercise_args.next() {
                let exercise_id = into_usize(exercise_id)?;
                let exercise_path = exercise_args.next().unwrap(); // safe unwrap because each --exercise takes 2 arguments
                let exercise_path = PathBuf::from(exercise_path);
                exercises.push((exercise_id, exercise_path));
            }

            client
                .download_or_update_exercises(exercises)
                .context("Failed to download exercises")?;

            let output = Output::finished_with_data("downloaded or updated exercises", None);
            print_output(&output, pretty)?
        }
        ("get-course-data", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;
            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;
            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;
            let data = CombinedCourseData {
                details,
                exercises,
                settings,
            };

            let output = Output::finished_with_data(
                "fetched course data",
                Data::CombinedCourseData(Box::new(data)),
            );
            print_output(&output, pretty)?
        }
        ("get-course-details", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;

            let output =
                Output::finished_with_data("fetched course details", Data::CourseDetails(details));
            print_output(&output, pretty)?
        }
        ("get-course-exercises", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;

            let output = Output::finished_with_data(
                "fetched course exercises",
                Data::CourseExercises(exercises),
            );
            print_output(&output, pretty)?
        }
        ("get-course-settings", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;

            let output =
                Output::finished_with_data("fetched course settings", Data::CourseData(settings));
            print_output(&output, pretty)?
        }
        ("get-courses", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let courses = client
                .list_courses(organization_slug)
                .context("Failed to get courses")?;

            let output = Output::finished_with_data("fetched courses", Data::Courses(courses));
            print_output(&output, pretty)?
        }
        ("get-exercise-details", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let course = client
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;

            let output = Output::finished_with_data(
                "fetched exercise details",
                Data::ExerciseDetails(course),
            );
            print_output(&output, pretty)?
        }
        ("get-exercise-submissions", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let submissions = client
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;

            let output = Output::finished_with_data(
                "fetched exercise submissions",
                Data::Submissions(submissions),
            );
            print_output(&output, pretty)?
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

            let update_result = client
                .get_exercise_updates(course_id, checksums)
                .context("Failed to get exercise updates")?;

            let output = Output::finished_with_data(
                "fetched exercise updates",
                Data::UpdateResult(update_result),
            );
            print_output(&output, pretty)?
        }
        ("get-organization", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let org = client
                .get_organization(organization_slug)
                .context("Failed to get organization")?;

            let output =
                Output::finished_with_data("fetched organization", Data::Organization(org));
            print_output(&output, pretty)?
        }
        ("get-organizations", Some(_matches)) => {
            let orgs = client
                .get_organizations()
                .context("Failed to get organizations")?;

            let output =
                Output::finished_with_data("fetched organizations", Data::Organizations(orgs));
            print_output(&output, pretty)?
        }
        ("get-unread-reviews", Some(matches)) => {
            let reviews_url = matches.value_of("reviews-url").unwrap();
            let reviews_url = into_url(reviews_url)?;

            let reviews = client
                .get_unread_reviews(reviews_url)
                .context("Failed to get unread reviews")?;

            let output =
                Output::finished_with_data("fetched unread reviews", Data::Reviews(reviews));
            print_output(&output, pretty)?
        }
        ("logged-in", Some(_matches)) => {
            if let Some(credentials) = credentials {
                let output = Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: "currently logged in".to_string(),
                    result: OutputResult::LoggedIn,
                    data: Some(Data::Token(credentials.token())),
                });
                print_output(&output, pretty)?
            } else {
                let output = Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: "currently not logged in".to_string(),
                    result: OutputResult::NotLoggedIn,
                    data: None,
                });
                print_output(&output, pretty)?
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
                client
                    .authenticate(client_name, email.to_string(), decoded)
                    .context("Failed to authenticate with TMC")?
            } else {
                unreachable!("validation error");
            };

            // create token file
            Credentials::save(client_name, token)?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: "logged in".to_string(),
                result: OutputResult::LoggedIn,
                data: None,
            });
            print_output(&output, pretty)?
        }
        ("logout", Some(_matches)) => {
            if let Some(credentials) = credentials.take() {
                credentials.remove()?;
            }

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: "logged out".to_string(),
                result: OutputResult::LoggedOut,
                data: None,
            });
            print_output(&output, pretty)?
        }
        ("mark-review-as-read", Some(matches)) => {
            let review_update_url = matches.value_of("review-update-url").unwrap();

            client
                .mark_review_as_read(review_update_url.to_string())
                .context("Failed to mark review as read")?;

            let output = Output::finished_with_data("marked review as read", None);
            print_output(&output, pretty)?
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

            file_util::lock!(submission_path);

            let new_submission = client
                .paste(
                    submission_url,
                    submission_path,
                    paste_message.map(str::to_string),
                    locale,
                )
                .context("Failed to get paste with comment")?;

            let output =
                Output::finished_with_data("sent paste", Data::NewSubmission(new_submission));
            print_output(&output, pretty)?
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

            file_util::lock!(submission_path);

            let new_submission = client
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .context("Failed to request code review")?;

            let output = Output::finished_with_data(
                "requested code review",
                Data::NewSubmission(new_submission),
            );
            print_output(&output, pretty)?
        }
        ("reset-exercise", Some(matches)) => {
            let save_old_state = matches.is_present("save-old-state");

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = PathBuf::from(exercise_path);

            let submission_url = matches.value_of("submission-url");

            file_util::lock!(&exercise_path);

            if save_old_state {
                // submit current state
                let submission_url = into_url(submission_url.unwrap())?;
                client.submit(submission_url, &exercise_path, None)?;
            }

            // reset exercise
            client.reset(exercise_id, exercise_path)?;

            let output = Output::finished_with_data("reset exercise", None);
            print_output(&output, pretty)?
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

            let response = client
                .send_feedback(feedback_url, feedback)
                .context("Failed to send feedback")?;

            let output = Output::finished_with_data(
                "sent feedback",
                Data::SubmissionFeedbackResponse(response),
            );
            print_output(&output, pretty)?
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

            file_util::lock!(submission_path);

            let new_submission = client
                .submit(submission_url, submission_path, locale)
                .context("Failed to submit")?;

            if dont_block {
                let output = Output::finished_with_data(
                    "submit exercise",
                    Data::NewSubmission(new_submission),
                );
                print_output(&output, pretty)?
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url;
                let submission_finished = client
                    .wait_for_submission(&submission_url)
                    .context("Failed while waiting for submissions")?;

                let output = Output::finished_with_data(
                    "submit exercise",
                    Data::SubmissionFinished(submission_finished),
                );
                print_output(&output, pretty)?
            }
        }
        ("update-exercises", Some(_)) => {
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
                    .get_exercises_details(exercise_ids)?
                    .into_iter()
                    .map(|e| (e.id, e))
                    .collect::<HashMap<_, _>>();
                for local_exercise in local_exercises {
                    let server_exercise =
                        server_exercises.get(&local_exercise.id).with_context(|| {
                            format!(
                                "Server did not return details for local exercise with id {}",
                                local_exercise.id
                            )
                        })?;
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
                        to_be_downloaded.push(DownloadOrUpdateCourseExercise {
                            course_slug: server_exercise.course_name.clone(),
                            exercise_slug: server_exercise.exercise_name.clone(),
                            path: target,
                        });
                    } else {
                        to_be_skipped.push(DownloadOrUpdateCourseExercise {
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

            let data = DownloadOrUpdateCourseExercisesResult {
                downloaded: to_be_downloaded,
                skipped: to_be_skipped,
            };
            let output = Output::finished_with_data(
                "downloaded or updated exercises",
                Data::ExerciseDownload(data),
            );
            print_output(&output, pretty)?
        }
        ("wait-for-submission", Some(matches)) => {
            let submission_url = matches.value_of("submission-url").unwrap();

            let submission_finished = client
                .wait_for_submission(submission_url)
                .context("Failed while waiting for submissions")?;

            let output = Output::finished_with_data(
                "finished waiting for submission",
                Data::SubmissionFinished(submission_finished),
            );
            print_output(&output, pretty)?
        }
        _ => unreachable!(),
    };

    Ok(printed)
}

fn run_settings(matches: &ArgMatches, pretty: bool) -> Result<PrintToken> {
    let client_name = matches.value_of("client-name").unwrap();

    let config_path = TmcConfig::get_location(client_name)?;
    let mut tmc_config = TmcConfig::load(client_name, &config_path)?;

    match matches.subcommand() {
        ("get", Some(matches)) => {
            let key = matches.value_of("setting").unwrap();
            let value: ConfigValue<'static> = tmc_config.get(key).into_owned();
            let output = Output::finished_with_data("retrieved value", Data::ConfigValue(value));
            print_output(&output, pretty)
        }
        ("list", Some(_)) => {
            let output =
                Output::finished_with_data("retrieved settings", Data::TmcConfig(tmc_config));
            print_output(&output, pretty)
        }
        ("migrate", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let course_slug = matches.value_of("course-slug").unwrap();

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let exercise_slug = matches.value_of("exercise-slug").unwrap();

            let exercise_checksum = matches.value_of("exercise-checksum").unwrap();

            config::migrate(
                &tmc_config,
                course_slug,
                exercise_slug,
                exercise_id,
                exercise_checksum,
                exercise_path,
            )?;

            let output = Output::finished_with_data("migrated exercise", None);
            print_output(&output, pretty)
        }
        ("move-projects-dir", Some(matches)) => {
            let dir = matches.value_of("dir").unwrap();
            let target = PathBuf::from(dir);

            config::move_projects_dir(tmc_config, &config_path, target)?;

            let output = Output::finished_with_data("moved project directory", None);
            print_output(&output, pretty)
        }
        ("set", Some(matches)) => {
            let key = matches.value_of("key").unwrap();
            let value = matches.value_of("json").unwrap();

            let value = match serde_json::from_str(value) {
                Ok(json) => json,
                Err(_) => {
                    // interpret as string
                    JsonValue::String(value.to_string())
                }
            };
            let value = json_to_toml(value)?;

            tmc_config
                .insert(key.to_string(), value.clone())
                .with_context(|| format!("Failed to set {} to {}", key, value))?;
            tmc_config.save(&config_path)?;

            let output = Output::finished_with_data("set setting", None);
            print_output(&output, pretty)
        }
        ("reset", Some(_)) => {
            TmcConfig::reset(client_name)?;

            let output = Output::finished_with_data("reset settings", None);
            print_output(&output, pretty)
        }
        ("unset", Some(matches)) => {
            let key = matches.value_of("setting").unwrap();
            tmc_config
                .remove(key)
                .with_context(|| format!("Failed to unset {}", key))?;
            tmc_config.save(&config_path)?;

            let output = Output::finished_with_data("unset setting", None);
            print_output(&output, pretty)
        }
        _ => unreachable!("validation error"),
    }
}

fn print_output(output: &Output, pretty: bool) -> Result<PrintToken> {
    print_output_with_file(output, pretty, None)
}

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
) -> Result<()> {
    let mut output_file = file_util::create_file_lock(output_path).with_context(|| {
        format!(
            "Failed to create results JSON file at {}",
            output_path.display()
        )
    })?;
    let guard = output_file.lock()?;

    if pretty {
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

fn json_to_toml(json: JsonValue) -> Result<TomlValue> {
    match json {
        JsonValue::Array(arr) => {
            let mut v = vec![];
            for value in arr {
                v.push(json_to_toml(value)?);
            }
            Ok(TomlValue::Array(v))
        }
        JsonValue::Bool(b) => Ok(TomlValue::Boolean(b)),
        JsonValue::Null => anyhow::bail!("The settings file cannot contain null values"),
        JsonValue::Number(num) => {
            if let Some(int) = num.as_i64() {
                Ok(TomlValue::Integer(int))
            } else if let Some(float) = num.as_f64() {
                Ok(TomlValue::Float(float))
            } else {
                // this error can occur because serde_json supports u64 ints but toml doesn't
                anyhow::bail!("The given number was too high: {}", num)
            }
        }
        JsonValue::Object(obj) => {
            let mut map = TomlMap::new();
            for (key, value) in obj {
                map.insert(key, json_to_toml(value)?);
            }
            Ok(TomlValue::Table(map))
        }
        JsonValue::String(s) => Ok(TomlValue::String(s)),
    }
}

fn move_dir(source: &Path, source_lock: FileLockGuard, target: &Path) -> anyhow::Result<()> {
    let mut file_count_copied = 0;
    let mut file_count_total = 0;
    for entry in WalkDir::new(source) {
        let entry =
            entry.with_context(|| format!("Failed to read file inside {}", source.display()))?;
        if entry.path().is_file() {
            file_count_total += 1;
        }
    }
    start_stage(
        file_count_total + 1,
        format!("Moving dir {} -> {}", source.display(), target.display()),
    );

    for entry in WalkDir::new(source).contents_first(true).min_depth(1) {
        let entry =
            entry.with_context(|| format!("Failed to read file inside {}", source.display()))?;
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
            let relative = entry_path.strip_prefix(source).unwrap();
            let target_path = target.join(relative);
            log::debug!(
                "Moving {} -> {}",
                entry_path.display(),
                target_path.display()
            );

            // create parent dir for target and copy it, remove source file after
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create directory at {}", parent.display())
                })?;
            }
            fs::copy(entry_path, &target_path).with_context(|| {
                format!(
                    "Failed to copy file from {} to {}",
                    entry_path.display(),
                    target_path.display()
                )
            })?;
            fs::remove_file(entry_path).with_context(|| {
                format!(
                    "Failed to remove file at {} after copying it",
                    entry_path.display()
                )
            })?;

            file_count_copied += 1;
            progress_stage(format!(
                "Moved file {} / {}",
                file_count_copied, file_count_total
            ));
        } else if entry_path.is_dir() {
            log::debug!("Deleting {}", entry_path.display());
            fs::remove_dir(entry_path).with_context(|| {
                format!("Failed to remove directory at {}", entry_path.display())
            })?;
        }
    }

    drop(source_lock);
    fs::remove_dir(source)?;

    finish_stage("Finished moving project directory");
    Ok(())
}

struct PrintToken;

fn start_stage(steps: usize, message: impl Into<String>) {
    progress_reporter::start_stage::<()>(steps, message.into(), None)
}

fn progress_stage(message: impl Into<String>) {
    progress_reporter::progress_stage::<()>(message.into(), None)
}

fn finish_stage(message: impl Into<String>) {
    progress_reporter::finish_stage::<()>(message.into(), None)
}
