//! CLI client for TMC

mod app;
mod config;
mod error;
mod output;

use self::config::ProjectsConfig;
use self::config::{CourseConfig, Credentials, Exercise, TmcConfig};
use self::error::{InvalidTokenError, SandboxTestError};
use self::output::{
    CombinedCourseData, DownloadOrUpdateCourseExercise, DownloadOrUpdateCourseExercisesResult,
    ErrorData, Kind, LocalExercise, Output, OutputData, OutputResult, Status, UpdatedExercise,
    Warnings,
};
use anyhow::{Context, Result};
use clap::{ArgMatches, Error, ErrorKind};
use heim::disk;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt::Debug;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tmc_client::oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
use tmc_client::{ClientError, ClientUpdateData, FeedbackAnswer, TmcClient, Token};
use tmc_langs_framework::{domain::StyleValidationResult, error::CommandError};
use tmc_langs_util::{
    progress_reporter::ProgressReporter,
    task_executor::{
        self, Course, GroupBits, ModeBits, Options, RefreshExercise, SourceBackend, TmcParams,
    },
    Language, OutputFormat,
};
use toml::{map::Map as TomlMap, Value as TomlValue};
use url::Url;
use walkdir::WalkDir;

#[quit::main]
fn main() {
    env_logger::init();

    let matches = app::create_app().get_matches();
    let pretty = matches.is_present("pretty");
    let mut warnings = vec![];

    if let Err(e) = run_app(matches, pretty, &mut warnings) {
        if print_warnings(pretty, &warnings).is_err() {
            // No need to handle the error; printing the actual error is more important
            log::error!("Failed to print warnings");
        }

        // error handling
        let causes: Vec<String> = e.chain().map(|e| format!("Caused by: {}", e)).collect();
        let message = error_message_special_casing(&e);
        let kind = solve_error_kind(&e);
        let sandbox_path = check_sandbox_err(&e);
        let error_output = Output::OutputData(OutputData {
            status: Status::Finished,
            message: Some(message),
            result: OutputResult::Error,
            data: Some(ErrorData {
                kind,
                trace: causes,
            }),
            percent_done: 1.0,
        });
        if let Err(err) = print_output_with_file(&error_output, pretty, sandbox_path, &warnings) {
            // the above function shouldn't fail ever, but in theory some data could
            // have a flawed Serialize implementation, so better safe than sorry
            let output = Output::OutputData::<()>(OutputData {
                status: Status::Crashed,
                message: Some(err.to_string()),
                result: OutputResult::Error,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings).expect("this should never fail");
        }
        quit::with_code(1);
    }
}

/// Goes through the error chain and checks for special error types that should be indicated by the Kind.
fn solve_error_kind(e: &anyhow::Error) -> Kind {
    for cause in e.chain() {
        // check for invalid token
        if cause.downcast_ref::<InvalidTokenError>().is_some() {
            return Kind::InvalidToken;
        }

        // check for http errors
        if let Some(ClientError::HttpError {
            url: _,
            status,
            error: _,
            obsolete_client,
        }) = cause.downcast_ref::<ClientError>()
        {
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
        if let Some(ClientError::NotLoggedIn) = cause.downcast_ref::<ClientError>() {
            return Kind::NotLoggedIn;
        }
        // check for connection error
        if let Some(ClientError::ConnectionError(..)) = cause.downcast_ref::<ClientError>() {
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

/// Goes through the error chain and returns the error output file path if a sandbox test error is found
fn check_sandbox_err(e: &anyhow::Error) -> Option<PathBuf> {
    for cause in e.chain() {
        // command not found errors are special cased to notify the user that they may need to install additional software
        if let Some(SandboxTestError {
            path: Some(path), ..
        }) = cause.downcast_ref::<SandboxTestError>()
        {
            return Some(path.clone());
        }
    }
    None
}

fn run_app(matches: ArgMatches, pretty: bool, warnings: &mut Vec<anyhow::Error>) -> Result<()> {
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some("ran checkstyle".to_string()),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: check_result,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("clean", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            task_executor::clean(exercise_path).with_context(|| {
                format!("Failed to clean exercise at {}", exercise_path.display(),)
            })?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!("cleaned exercise at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "compressed project from {} to {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            match run_core(
                client,
                client_name,
                &mut credentials,
                matches,
                pretty,
                warnings,
            ) {
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

            let usage = smol::block_on(disk::usage(path)).with_context(|| {
                format!("Failed to get disk usage from path {}", path.display())
            })?;
            let free = usage.free().get::<heim::units::information::megabyte>();

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "calculated free disk space for partition containing {}",
                    path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(free),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("extract-project", Some(matches)) => {
            let archive_path = matches.value_of("archive-path").unwrap();

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let archive = File::open(archive_path)
                .with_context(|| format!("Failed to open file at {}", archive_path))?;
            task_executor::extract_project(archive, output_path, true).with_context(|| {
                format!("Failed to extract project at {}", output_path.display())
            })?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "extracted project from {} to {}",
                    archive_path,
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("fast-available-points", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let points = task_executor::get_available_points(exercise_path)?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("found {} available points", points.len())),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(points),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("find-exercises", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let exercises =
                task_executor::find_exercise_directories(exercise_path).with_context(|| {
                    format!(
                        "Failed to find exercise directories in {}",
                        exercise_path.display(),
                    )
                })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&exercises, output_path, pretty)?;
            }

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("found exercises at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(exercises),
            });
            print_output(&output, pretty, &warnings)?
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
                write_result_to_file_as_json(&config, output_path, pretty)?;
            }

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "created exercise packaging config from {}",
                    exercise_path.display(),
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(config),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("list-local-course-exercises", Some(matches)) => {
            let client_name = matches.value_of("client-name").unwrap();

            let course_slug = matches.value_of("course-slug").unwrap();

            let projects_dir = TmcConfig::load(client_name)?.projects_dir;
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("listed local exercises for {}", course_slug,)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(local_exercises),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("prepare-solutions", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            task_executor::prepare_solution(exercise_path, output_path).with_context(|| {
                format!(
                    "Failed to prepare solutions for exercise at {}",
                    exercise_path.display(),
                )
            })?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "prepared solutions for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("prepare-stubs", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            task_executor::prepare_stub(exercise_path, output_path).with_context(|| {
                format!(
                    "Failed to prepare stubs for exercise at {}",
                    exercise_path.display(),
                )
            })?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "prepared stubs for {} at {}",
                    exercise_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: Some(format!(
                    "prepared submission for {} at {}",
                    submission_path.display(),
                    output_path.display()
                )),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("refresh-course", Some(matches)) => {
            let course_name = matches.value_of("course-name").unwrap();
            let cache_path = matches.value_of("cache-path").unwrap();
            let clone_path = matches.value_of("clone-path").unwrap();
            let stub_path = matches.value_of("stub-path").unwrap();
            let stub_zip_path = matches.value_of("stub-zip-path").unwrap();
            let solution_path = matches.value_of("solution-path").unwrap();
            let solution_zip_path = matches.value_of("solution-zip-path").unwrap();
            let exercise_args = matches.values_of("exercise");
            let source_backend = matches.value_of("source-backend").unwrap();
            let source_url = matches.value_of("source-url").unwrap();
            let git_branch = matches.value_of("git-branch").unwrap();
            let no_directory_changes = matches.is_present("no-directory-changes");
            let no_background_operations = matches.is_present("no-background-operations");
            let chmod_bits = matches.value_of("chmod-bits");
            let chgrp_uid = matches.value_of("chgrp-uid");
            let cache_root = matches.value_of("cache-root").unwrap();
            let rails_root = matches.value_of("rails-root").unwrap();

            let mut exercises = vec![];
            if let Some(mut exercise_args) = exercise_args {
                while let Some(exercise_name) = exercise_args.next() {
                    let relative_path = exercise_args.next().unwrap();
                    let available_points: Vec<_> =
                        exercise_args.next().unwrap().split(',').collect();
                    exercises.push(RefreshExercise {
                        name: exercise_name.to_string(),
                        relative_path: PathBuf::from(relative_path),
                        available_points: available_points
                            .into_iter()
                            .map(str::to_string)
                            .filter(|s| !s.is_empty())
                            .collect(),
                    });
                }
            }
            let source_backend = match source_backend {
                "git" => SourceBackend::Git,
                _ => unreachable!("validation error"),
            };
            let course = Course {
                name: course_name.to_string(),
                cache_path: PathBuf::from(cache_path),
                clone_path: PathBuf::from(clone_path),
                stub_path: PathBuf::from(stub_path),
                stub_zip_path: PathBuf::from(stub_zip_path),
                solution_path: PathBuf::from(solution_path),
                solution_zip_path: solution_zip_path.into(),
                exercises,
                source_backend,
                source_url: source_url.to_string(),
                git_branch: git_branch.to_string(),
            };
            let options = Options {
                no_background_operations,
                no_directory_changes,
            };
            let chmod_bits = if let Some(chmod_bits) = chmod_bits {
                Some(ModeBits::from_str_radix(chmod_bits, 8).with_context(|| {
                    format!("Failed to convert chmod bits to an integer: {}", chmod_bits,)
                })?)
            } else {
                None
            };
            let chgrp_uid = if let Some(chgrp_uid) = chgrp_uid {
                Some(GroupBits::from_str_radix(chgrp_uid, 10).with_context(|| {
                    format!("Failed to convert chgrp UID to an integer: {}", chgrp_uid,)
                })?)
            } else {
                None
            };

            let refresh_result = task_executor::refresh_course(
                course,
                options,
                chmod_bits,
                chgrp_uid,
                PathBuf::from(cache_root),
                PathBuf::from(rails_root),
                move |update| {
                    let output = Output::StatusUpdate(update);
                    print_output(&output, pretty, &[])?;
                    Ok(())
                },
            )
            .with_context(|| format!("Failed to refresh course {}", course_name))?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("refreshed course {}", course_name)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(refresh_result),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("run-tests", Some(matches)) => {
            let checkstyle_output_path = matches.value_of("checkstyle-output-path");
            let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale");

            let output_path = matches.value_of("output-path");
            let output_path = output_path.map(Path::new);

            let test_result =
                task_executor::run_tests(exercise_path, warnings).with_context(|| {
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("ran tests for {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(test_result),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("settings", Some(matches)) => run_settings(matches, pretty, &warnings)?,
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
                task_executor::scan_exercise(exercise_path, exercise_name.to_string(), warnings)
                    .with_context(|| {
                        format!("Failed to scan exercise at {}", exercise_path.display())
                    })?;

            if let Some(output_path) = output_path {
                write_result_to_file_as_json(&scan_result, output_path, pretty)?;
            }

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: Some(format!("scanned exercise at {}", exercise_path.display(),)),
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(scan_result),
            });
            print_output(&output, pretty, &warnings)?
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
    warnings: &mut Vec<anyhow::Error>,
) -> Result<PrintToken> {
    // set progress report to print the updates to stdout as JSON
    client.set_progress_reporter(ProgressReporter::new(move |update| {
        let output = Output::StatusUpdate::<ClientUpdateData>(update);
        print_output(&output, pretty, &[])?;
        Ok(())
    }))?;

    // proof of having printed the output
    let printed: PrintToken = match matches.subcommand() {
        ("check-exercise-updates", Some(_)) => {
            let mut updated_exercises = vec![];

            let projects_dir = TmcConfig::load(client_name)?.projects_dir;
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(updated_exercises),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("download-model-solution", Some(matches)) => {
            let solution_download_url = matches.value_of("solution-download-url").unwrap();
            let solution_download_url = into_url(solution_download_url)?;

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            client
                .download_model_solution(solution_download_url, target)
                .context("Failed to download model solution")?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            // increment steps for reset
            client.increment_progress_steps();
            if save_old_state {
                // submit old exercise
                let submission_url = into_url(submission_url.unwrap())?;
                // increment steps for submit
                client.increment_progress_steps();
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
            task_executor::extract_student_files(temp_zip, &output_path)?;
            log::debug!("extracted project");

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("download-or-update-course-exercises", Some(matches)) => {
            let exercise_ids = matches.values_of("exercise-id").unwrap();

            // collect exercise into (id, path) pairs
            let exercises = exercise_ids
                .into_iter()
                .map(into_usize)
                .collect::<Result<_>>()?;
            let exercises_details = client.get_exercises_details(exercises)?;

            let projects_dir = TmcConfig::load(client_name)?.projects_dir;
            let mut projects_config = ProjectsConfig::load(&projects_dir)?;

            let mut course_data = HashMap::<String, Vec<(String, String, usize)>>::new();
            let mut exercises_and_paths = vec![];
            let mut downloaded = vec![];
            let mut skipped = vec![];
            for exercise_detail in exercises_details {
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
                            skipped.push(DownloadOrUpdateCourseExercise {
                                course_slug: exercise_detail.course_name,
                                exercise_slug: exercise_detail.exercise_name,
                            });
                            continue;
                        }
                    }
                }
                // not skipped, will be downloaded
                // if any download fails, an error is returned instead, so it's ok to just push them to downloaded here
                downloaded.push(DownloadOrUpdateCourseExercise {
                    course_slug: exercise_detail.course_name.clone(),
                    exercise_slug: exercise_detail.exercise_name.clone(),
                });

                let target = ProjectsConfig::get_exercise_download_target(
                    &projects_dir,
                    &exercise_detail.course_name,
                    &exercise_detail.exercise_name,
                );

                let entry = course_data.entry(exercise_detail.course_name);
                let course_exercises = entry.or_default();
                course_exercises.push((
                    exercise_detail.exercise_name,
                    exercise_detail.checksum,
                    exercise_detail.id,
                ));

                exercises_and_paths.push((exercise_detail.id, target));
            }
            client
                .download_or_update_exercises(exercises_and_paths)
                .context("Failed to download exercises")?;

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

            let data = DownloadOrUpdateCourseExercisesResult {
                downloaded,
                skipped,
            };
            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(data),
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(data),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-course-details", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let details = client
                .get_course_details(course_id)
                .context("Failed to get course details")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(details),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-course-exercises", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let exercises = client
                .get_course_exercises(course_id)
                .context("Failed to get course")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(exercises),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-course-settings", Some(matches)) => {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let settings = client
                .get_course(course_id)
                .context("Failed to get course")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(settings),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-courses", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let courses = client
                .list_courses(organization_slug)
                .context("Failed to get courses")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(courses),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-exercise-details", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let course = client
                .get_exercise_details(exercise_id)
                .context("Failed to get course")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(course),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-exercise-submissions", Some(matches)) => {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let submissions = client
                .get_exercise_submissions_for_current_user(exercise_id)
                .context("Failed to get submissions")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(submissions),
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(update_result),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-organization", Some(matches)) => {
            let organization_slug = matches.value_of("organization").unwrap();

            let org = client
                .get_organization(organization_slug)
                .context("Failed to get organization")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(org),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-organizations", Some(_matches)) => {
            let orgs = client
                .get_organizations()
                .context("Failed to get organizations")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(orgs),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("get-unread-reviews", Some(matches)) => {
            let reviews_url = matches.value_of("reviews-url").unwrap();
            let reviews_url = into_url(reviews_url)?;

            let reviews = client
                .get_unread_reviews(reviews_url)
                .context("Failed to get unread reviews")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: Some(reviews),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("logged-in", Some(_matches)) => {
            if let Some(credentials) = credentials {
                let output = Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::LoggedIn,
                    percent_done: 1.0,
                    data: Some(credentials.token()),
                });
                print_output(&output, pretty, &warnings)?
            } else {
                let output = Output::OutputData::<()>(OutputData {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::NotLoggedIn,
                    percent_done: 1.0,
                    data: None,
                });
                print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedIn,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("logout", Some(_matches)) => {
            if let Some(credentials) = credentials.take() {
                credentials.remove()?;
            }

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("mark-review-as-read", Some(matches)) => {
            let review_update_url = matches.value_of("reiew-update-url").unwrap();

            client
                .mark_review_as_read(review_update_url.to_string())
                .context("Failed to mark review as read")?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
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

            let new_submission = client
                .paste(
                    submission_url,
                    submission_path,
                    paste_message.map(str::to_string),
                    locale,
                )
                .context("Failed to get paste with comment")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(new_submission),
            });
            print_output(&output, pretty, &warnings)?
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

            let new_submission = client
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .context("Failed to request code review")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(new_submission),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("reset-exercise", Some(matches)) => {
            let save_old_state = matches.is_present("save-old-state");

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = PathBuf::from(exercise_path);

            let submission_url = matches.value_of("submission-url");

            if save_old_state {
                // submit current state
                let submission_url = into_url(submission_url.unwrap())?;
                client.increment_progress_steps();
                client.submit(submission_url, &exercise_path, None)?;
            }

            // reset exercise
            client.reset(exercise_id, exercise_path)?;

            let output = Output::OutputData::<()>(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, &warnings)?
        }
        ("run-checkstyle", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let validation_result = client
                .run_checkstyle(exercise_path, locale)
                .context("Failed to run checkstyle")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(validation_result),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("run-tests", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let run_result = client
                .run_tests(exercise_path, warnings)
                .context("Failed to run tests")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(run_result),
            });
            print_output(&output, pretty, &warnings)?
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

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(response),
            });
            print_output(&output, pretty, &warnings)?
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
                client.increment_progress_steps();
            }
            let new_submission = client
                .submit(submission_url, submission_path, locale)
                .context("Failed to submit")?;

            if dont_block {
                let output = Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::SentData,
                    percent_done: 1.0,
                    data: Some(new_submission),
                });

                print_output(&output, pretty, &warnings)?
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url;
                let submission_finished = client
                    .wait_for_submission(&submission_url)
                    .context("Failed while waiting for submissions")?;

                let output = Output::OutputData(OutputData {
                    status: Status::Finished,
                    message: None,
                    result: OutputResult::RetrievedData,
                    percent_done: 1.0,
                    data: Some(submission_finished),
                });
                print_output(&output, pretty, &warnings)?
            }
        }
        ("update-exercises", Some(_)) => {
            let mut exercises_to_update = vec![];
            let mut downloaded_exercises = vec![];
            let mut skipped_exercises = vec![];
            let mut course_data = HashMap::<String, Vec<(String, String, usize)>>::new();

            let projects_dir = TmcConfig::load(client_name)?.projects_dir;
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
                    if server_exercise.checksum != local_exercise.checksum {
                        // server has an updated exercise
                        let target = ProjectsConfig::get_exercise_download_target(
                            &projects_dir,
                            &server_exercise.course_name,
                            &server_exercise.exercise_name,
                        );
                        let exercise_list = course_data
                            .entry(server_exercise.course_name.clone())
                            .or_default();
                        exercise_list.push((
                            server_exercise.exercise_name.clone(),
                            server_exercise.checksum.clone(),
                            server_exercise.id,
                        ));
                        exercises_to_update.push((local_exercise.id, target));
                        downloaded_exercises.push(DownloadOrUpdateCourseExercise {
                            course_slug: server_exercise.course_name.clone(),
                            exercise_slug: server_exercise.exercise_name.clone(),
                        });
                    } else {
                        skipped_exercises.push(DownloadOrUpdateCourseExercise {
                            course_slug: server_exercise.course_name.clone(),
                            exercise_slug: server_exercise.exercise_name.clone(),
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
                downloaded: downloaded_exercises,
                skipped: skipped_exercises,
            };
            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(data),
            });
            print_output(&output, pretty, &warnings)?
        }
        ("wait-for-submission", Some(matches)) => {
            let submission_url = matches.value_of("submission-url").unwrap();

            let submission_finished = client
                .wait_for_submission(submission_url)
                .context("Failed while waiting for submissions")?;
            let submission_finished = serde_json::to_string(&submission_finished)
                .context("Failed to serialize submission results")?;

            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(submission_finished),
            });
            print_output(&output, pretty, &warnings)?
        }
        _ => unreachable!(),
    };

    Ok(printed)
}

fn run_settings(
    matches: &ArgMatches,
    pretty: bool,
    warnings: &[anyhow::Error],
) -> Result<PrintToken> {
    let client_name = matches.value_of("client-name").unwrap();
    let mut tmc_config = TmcConfig::load(client_name)?;

    match matches.subcommand() {
        ("get", Some(matches)) => {
            let key = matches.value_of("setting").unwrap();
            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::RetrievedData,
                message: Some("Retrieved value".to_string()),
                percent_done: 1.0,
                data: Some(tmc_config.get(key)),
            });
            print_output(&output, pretty, warnings)
        }
        ("list", Some(_)) => {
            let output = Output::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::RetrievedData,
                message: Some("Retrieved settings".to_string()),
                percent_done: 1.0,
                data: Some(tmc_config),
            });
            print_output(&output, pretty, warnings)
        }
        ("migrate", Some(matches)) => {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let course_slug = matches.value_of("course-slug").unwrap();

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let exercise_slug = matches.value_of("exercise-slug").unwrap();

            let exercise_checksum = matches.value_of("exercise-checksum").unwrap();

            let mut projects_config = ProjectsConfig::load(&tmc_config.projects_dir)?;
            let course_config = projects_config
                .courses
                .entry(course_slug.to_string())
                .or_insert(CourseConfig {
                    course: course_slug.to_string(),
                    exercises: BTreeMap::new(),
                });

            let target_dir = ProjectsConfig::get_exercise_download_target(
                &tmc_config.projects_dir,
                course_slug,
                exercise_slug,
            );
            if target_dir.exists() {
                anyhow::bail!("Tried to migrate exercise to {}; however, something already exists at that path.", target_dir.display());
            }

            course_config.exercises.insert(
                exercise_slug.to_string(),
                Exercise {
                    id: exercise_id,
                    checksum: exercise_checksum.to_string(),
                },
            );

            move_dir(exercise_path, &target_dir, pretty)?;
            course_config.save_to_projects_dir(&tmc_config.projects_dir)?;

            let output = Output::<()>::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::ExecutedCommand,
                message: Some("Migrated exercise".to_string()),
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, warnings)
        }
        ("move-projects-dir", Some(matches)) => {
            let dir = matches.value_of("dir").unwrap();
            let target = PathBuf::from(dir);

            if target.is_file() {
                anyhow::bail!("The target path points to a file.")
            }
            if !target.exists() {
                fs::create_dir_all(&target).with_context(|| {
                    format!("Failed to create directory at {}", target.display())
                })?;
            }

            let target_canon = target
                .canonicalize()
                .with_context(|| format!("Failed to canonicalize {}", target.display()))?;
            let prev_dir_canon = tmc_config.projects_dir.canonicalize().with_context(|| {
                format!(
                    "Failed to canonicalize {}",
                    tmc_config.projects_dir.display()
                )
            })?;
            if target_canon == prev_dir_canon {
                anyhow::bail!(
                    "Attempted to move the projects-dir to the directory it's already in."
                )
            }

            let old_projects_dir = tmc_config.set_projects_dir(target.clone())?;
            move_dir(&old_projects_dir, &target, pretty)?;
            tmc_config.save(client_name)?;

            let output = Output::<()>::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::ExecutedCommand,
                message: Some("Moved project directory".to_string()),
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, warnings)
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
            tmc_config.save(client_name)?;

            let output = Output::<()>::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::ExecutedCommand,
                message: Some("Set setting".to_string()),
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, warnings)
        }
        ("reset", Some(_)) => {
            TmcConfig::reset(client_name)?;

            let output = Output::<()>::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::ExecutedCommand,
                message: Some("Reset settings".to_string()),
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, warnings)
        }
        ("unset", Some(matches)) => {
            let key = matches.value_of("setting").unwrap();
            tmc_config
                .remove(key)
                .with_context(|| format!("Failed to unset {}", key))?;
            tmc_config.save(client_name)?;

            let output = Output::<()>::OutputData(OutputData {
                status: Status::Finished,
                result: OutputResult::ExecutedCommand,
                message: Some("Unset setting".to_string()),
                percent_done: 1.0,
                data: None,
            });
            print_output(&output, pretty, warnings)
        }
        _ => unreachable!("validation error"),
    }
}

fn print_output<T: Serialize + Debug>(
    output: &Output<T>,
    pretty: bool,
    warnings: &[anyhow::Error],
) -> Result<PrintToken> {
    print_output_with_file(output, pretty, None, warnings)
}

fn print_output_with_file<T: Serialize + Debug>(
    output: &Output<T>,
    pretty: bool,
    path: Option<PathBuf>,
    warnings: &[anyhow::Error],
) -> Result<PrintToken> {
    print_warnings(pretty, warnings)?;

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

fn print_warnings(pretty: bool, warnings: &[anyhow::Error]) -> Result<()> {
    if warnings.is_empty() {
        return Ok(());
    }

    let warnings_output = Output::<()>::Warnings(Warnings::from_error_list(warnings));
    let warnings_json = if pretty {
        serde_json::to_string_pretty(&warnings_output)
    } else {
        serde_json::to_string(&warnings_output)
    }
    .with_context(|| format!("Failed to convert {:?} to JSON", warnings_output))?;
    println!("{}", warnings_json);
    Ok(())
}

fn write_result_to_file_as_json<T: Serialize>(
    result: &T,
    output_path: &Path,
    pretty: bool,
) -> Result<()> {
    let output_file = File::create(output_path).with_context(|| {
        format!(
            "Failed to create results JSON file at {}",
            output_path.display()
        )
    })?;

    if pretty {
        serde_json::to_writer_pretty(output_file, result).with_context(|| {
            format!(
                "Failed to write result as JSON to {}",
                output_path.display()
            )
        })?;
    } else {
        serde_json::to_writer(output_file, result).with_context(|| {
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

fn move_dir(source: &Path, target: &Path, pretty: bool) -> anyhow::Result<()> {
    let reporter = ProgressReporter::new(move |update| {
        let output = Output::StatusUpdate::<()>(update);
        print_output(&output, pretty, &[])?;
        Ok(())
    });

    reporter
        .progress(
            format!("Moving dir {} -> {}", source.display(), target.display()),
            0.0,
            None,
        )
        .map_err(|e| anyhow::anyhow!(e))?;

    let mut file_count_copied = 0;
    let mut file_count_total = 0;
    for entry in WalkDir::new(source) {
        let entry =
            entry.with_context(|| format!("Failed to read file inside {}", source.display()))?;
        if entry.path().is_file() {
            file_count_total += 1;
        }
    }
    for entry in WalkDir::new(source).contents_first(true) {
        let entry =
            entry.with_context(|| format!("Failed to read file inside {}", source.display()))?;
        let entry_path = entry.path();

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
            reporter
                .progress(
                    format!("Moved file {} / {}", file_count_copied, file_count_total),
                    file_count_copied as f64 / file_count_total as f64,
                    None,
                )
                .map_err(|e| anyhow::anyhow!(e))?;
        } else if entry_path.is_dir() {
            log::debug!("Deleting {}", entry_path.display());
            fs::remove_dir(entry_path).with_context(|| {
                format!("Failed to remove directory at {}", entry_path.display())
            })?;
        }
    }

    reporter
        .finish_step("Finished moving project directory", None)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

struct PrintToken;
