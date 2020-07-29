//! CLI client for TMC

mod app;
mod output;

use output::{Output, OutputResult, Status};

use anyhow::{Context, Result};
use clap::{Error, ErrorKind};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tmc_langs_core::oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
use tmc_langs_core::{FeedbackAnswer, StatusType, TmcCore, Token};
use tmc_langs_framework::io::submission_processing;
use tmc_langs_util::{
    task_executor::{self, TmcParams},
    Language,
};
use url::Url;
use walkdir::WalkDir;

#[quit::main]
fn main() {
    env_logger::init();

    if let Err(e) = run() {
        let mut causes = vec![];
        let mut next_source = e.source();
        while let Some(source) = next_source {
            causes.push(format!("Caused by: {}", source.to_string()));
            next_source = source.source();
        }
        let error_output = Output {
            status: Status::Crashed,
            message: Some(e.to_string()),
            result: OutputResult::Error,
            data: Some(causes),
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

fn run() -> Result<()> {
    let matches = app::create_app().get_matches_safe()?;

    // non-core
    // todo: print (generic?) success messages
    if let Some(matches) = matches.subcommand_matches("checkstyle") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let locale = matches.value_of("locale").unwrap();
        let locale = into_locale(locale)?;

        run_checkstyle(exercise_path, output_path, locale)?;

        let output = Output::<()> {
            status: Status::Successful,
            message: Some("ran checkstyle".to_string()),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: None,
        };
        print_output(&output)?
    } else if let Some(matches) = matches.subcommand_matches("compress-project") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let data = task_executor::compress_project(exercise_path).with_context(|| {
            format!("Failed to compress project at {}", exercise_path.display())
        })?;

        let mut output_file = File::create(output_path)
            .with_context(|| format!("Failed to create file at {}", output_path.display()))?;

        output_file.write_all(&data).with_context(|| {
            format!(
                "Failed to write compressed project to {}",
                output_path.display()
            )
        })?;

        let output = Output::<()> {
            status: Status::Successful,
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
    } else if let Some(matches) = matches.subcommand_matches("extract-project") {
        let archive_path = matches.value_of("archive-path").unwrap();
        let archive_path = Path::new(archive_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        task_executor::extract_project(archive_path, output_path)
            .with_context(|| format!("Failed to extract project at {}", output_path.display()))?;

        let output = Output::<()> {
            status: Status::Successful,
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
    } else if let Some(matches) = matches.subcommand_matches("prepare-solutions") {
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
            status: Status::Successful,
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
    } else if let Some(matches) = matches.subcommand_matches("prepare-stubs") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let exercises = find_exercise_directories(exercise_path);

        task_executor::prepare_stubs(exercises, exercise_path, output_path).with_context(|| {
            format!(
                "Failed to prepare stubs for exercise at {}",
                exercise_path.display(),
            )
        })?;

        let output = Output::<()> {
            status: Status::Successful,
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
    } else if let Some(matches) = matches.subcommand_matches("prepare-submission") {
        let submission_path = matches.value_of("submission-path").unwrap();
        let submission_path = Path::new(submission_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let tmc_params_values = matches.values_of("tmc-param").unwrap_or_default();
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
                tmc_params
                    .insert_string(key, values[0])
                    .context("invalid tmc-param key-value pair")?;
            } else {
                tmc_params
                    .insert_array(key, values)
                    .context("invalid tmc-param key-value pair")?;
            }
        }

        let clone_path = matches.value_of("clone-path").unwrap();
        let clone_path = Path::new(clone_path);

        let output_zip = matches.is_present("output-zip");

        let top_level_dir_name = matches.value_of("top-level-dir-name");
        let top_level_dir_name = top_level_dir_name.map(str::to_string);

        let stub_zip_path = matches.value_of("stub-zip-path");
        let stub_zip_path = stub_zip_path.map(Path::new);

        task_executor::prepare_submission(
            submission_path,
            output_path,
            top_level_dir_name,
            tmc_params,
            clone_path,
            stub_zip_path,
            output_zip,
        )?;

        let output = Output::<()> {
            status: Status::Successful,
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
    } else if let Some(matches) = matches.subcommand_matches("run-tests") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let checkstyle_output_path = matches.value_of("checkstyle-output-path");
        let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

        let test_result = task_executor::run_tests(exercise_path).with_context(|| {
            format!(
                "Failed to run tests for exercise at {}",
                exercise_path.display()
            )
        })?;

        write_result_to_file_as_json(&test_result, output_path)?;

        if let Some(checkstyle_output_path) = checkstyle_output_path {
            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            run_checkstyle(exercise_path, checkstyle_output_path, locale)?;
        }

        let output = Output {
            status: Status::Successful,
            message: Some(format!("ran tests for {}", exercise_path.display(),)),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: Some(test_result),
        };
        print_output(&output)?
    } else if let Some(matches) = matches.subcommand_matches("scan-exercise") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

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

        let scan_result = task_executor::scan_exercise(exercise_path, exercise_name.to_string())
            .with_context(|| format!("Failed to scan exercise at {}", exercise_path.display()))?;

        write_result_to_file_as_json(&scan_result, output_path)?;

        let output = Output {
            status: Status::Successful,
            message: Some(format!("scanned exercise at {}", exercise_path.display(),)),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: Some(scan_result),
        };
        print_output(&output)?
    } else if let Some(matches) = matches.subcommand_matches("find-exercises") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let mut exercises = vec![];
        // silently skips errors
        for entry in WalkDir::new(exercise_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() == "private")
            .filter(submission_processing::is_hidden_dir)
            .filter(submission_processing::contains_tmcignore)
        {
            log::debug!("processing {}", entry.path().display());
            // TODO: Java implementation doesn't scan root directories
            if task_executor::is_exercise_root_directory(entry.path()) {
                exercises.push(entry.into_path());
            }
        }

        write_result_to_file_as_json(&exercises, output_path)?;

        let output = Output {
            status: Status::Successful,
            message: Some(format!("found exercises at {}", exercise_path.display(),)),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: Some(exercises),
        };
        print_output(&output)?
    } else if let Some(matches) = matches.subcommand_matches("get-exercise-packaging-configuration")
    {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let config = task_executor::get_exercise_packaging_configuration(exercise_path)
            .with_context(|| {
                format!(
                    "Failed to get exercise packaging configuration for exercise at {}",
                    exercise_path.display(),
                )
            })?;

        write_result_to_file_as_json(&config, output_path)?;

        let output = Output {
            status: Status::Successful,
            message: Some(format!(
                "created exercise packaging config from {}",
                exercise_path.display(),
            )),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: Some(config),
        };
        print_output(&output)?
    } else if let Some(matches) = matches.subcommand_matches("clean") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        task_executor::clean(exercise_path)
            .with_context(|| format!("Failed to clean exercise at {}", exercise_path.display(),))?;

        let output = Output::<()> {
            status: Status::Successful,
            message: Some(format!("cleaned exercise at {}", exercise_path.display(),)),
            result: OutputResult::ExecutedCommand,
            percent_done: 1.0,
            data: None,
        };
        print_output(&output)?
    }

    // core
    // core commands should print their results using print_output
    if let Some(matches) = matches.subcommand_matches("core") {
        let root_url =
            env::var("TMC_LANGS_ROOT_URL").unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
        let client_name = matches.value_of("client-name").unwrap();
        let client_version = matches.value_of("client-version").unwrap();
        let mut core = TmcCore::new_in_config(
            root_url,
            client_name.to_string(),
            client_version.to_string(),
        )
        .context("Failed to create TmcCore")?;
        // set progress report to print the updates to stdout as JSON
        core.set_progress_report(|update| {
            // convert to output
            let output = Output::<()> {
                status: Status::InProgress,
                message: Some(update.message.to_string()),
                result: OutputResult::Core(update.status_type),
                percent_done: update.percent_done,
                data: None,
            };
            print_output(&output).map_err(|e| e.into())
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

        if let Some(matches) = matches.subcommand_matches("login") {
            let email = matches.value_of("email");
            let set_access_token = matches.value_of("set-access-token");
            let base64 = matches.is_present("base64");

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
                // TODO: "Please enter password" and quiet param
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
                status: Status::Successful,
                message: None,
                result: OutputResult::LoggedIn,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?;
        } else if let Some(_matches) = matches.subcommand_matches("logout") {
            if credentials_path.exists() {
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove credentials at {}",
                        credentials_path.display()
                    )
                })?;
            }

            let output = Output::<()> {
                status: Status::Successful,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?;
        } else if let Some(_matches) = matches.subcommand_matches("logged-in") {
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
                    status: Status::Successful,
                    message: None,
                    result: OutputResult::LoggedIn,
                    percent_done: 1.0,
                    data: Some(token),
                };
                print_output(&output)?;
            } else {
                let output = Output::<()> {
                    status: Status::Successful,
                    message: None,
                    result: OutputResult::NotLoggedIn,
                    percent_done: 1.0,
                    data: None,
                };
                print_output(&output)?;
            }
        } else if let Some(_matches) = matches.subcommand_matches("get-organizations") {
            let orgs = core
                .get_organizations()
                .context("Failed to get organizations")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(orgs),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("get-organization") {
            let organization_slug = matches.value_of("organization").unwrap();
            let org = core
                .get_organization(organization_slug)
                .context("Failed to get organization")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(org),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("download-or-update-exercises") {
            let mut exercise_args = matches.values_of("exercise").unwrap();
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
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: None,
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("get-course-details") {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let course_details = core
                .get_course_details(course_id)
                .context("Failed to get course details")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(course_details),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("get-courses") {
            let organization_slug = matches.value_of("organization").unwrap();
            let courses = core
                .list_courses(organization_slug)
                .context("Failed to get courses")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(courses),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("get-course-settings") {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;
            let course = core.get_course(course_id).context("Failed to get course")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(course),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("get-course-exercises") {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;
            let course = core
                .get_course_exercises(course_id)
                .context("Failed to get course")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(course),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("paste") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);
            let paste_message = matches.value_of("paste-message");

            let locale = matches.value_of("locale");
            let locale = if let Some(locale) = locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let new_submission = core
                .paste(
                    submission_url,
                    submission_path,
                    paste_message.map(str::to_string),
                    locale,
                )
                .context("Failed to get paste with comment")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(new_submission),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("run-checkstyle") {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);
            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let validation_result = core
                .run_checkstyle(exercise_path, locale)
                .context("Failed to run checkstyle")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(validation_result),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("run-tests") {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let run_result = core
                .run_tests(exercise_path)
                .context("Failed to run tests")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::ExecutedCommand,
                percent_done: 1.0,
                data: Some(run_result),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("send-feedback") {
            let feedback_url = matches.value_of("feedback-url").unwrap();
            let feedback_url = into_url(feedback_url)?;

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

            let response = core
                .send_feedback(feedback_url, feedback)
                .context("Failed to send feedback")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(response),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("submit") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let optional_locale = matches.value_of("locale");
            let optional_locale = if let Some(locale) = optional_locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let dont_block = matches.is_present("dont-block");

            let new_submission = core
                .submit(submission_url, submission_path, optional_locale)
                .context("Failed to submit")?;

            if dont_block {
                let output = Output {
                    status: Status::Successful,
                    message: None,
                    result: OutputResult::SentData,
                    percent_done: 1.0,
                    data: Some(new_submission),
                };

                print_output(&output)?;
            } else {
                // same as wait-for-submission
                let submission_url = new_submission.submission_url;
                let submission_finished = core
                    .wait_for_submission(&submission_url)
                    .context("Failed while waiting for submissions")?;
                let submission_finished = serde_json::to_string(&submission_finished)
                    .context("Failed to serialize submission results")?;

                let output = Output {
                    status: Status::Successful,
                    message: None,
                    result: OutputResult::RetrievedData,
                    percent_done: 1.0,
                    data: Some(submission_finished),
                };
                print_output(&output)?;
            }
        } else if let Some(matches) = matches.subcommand_matches("wait-for-submission") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_finished = core
                .wait_for_submission(submission_url)
                .context("Failed while waiting for submissions")?;
            let submission_finished = serde_json::to_string(&submission_finished)
                .context("Failed to serialize submission results")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(submission_finished),
            };
            print_output(&output)?;
        } else if let Some(_matches) = matches.subcommand_matches("get-exercise-updates") {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let mut exercise_checksums = matches.values_of("exercise").unwrap();
            let mut checksums = HashMap::new();
            while let Some(exercise_id) = exercise_checksums.next() {
                let exercise_id = into_usize(exercise_id)?;
                let checksum = exercise_checksums.next().unwrap();
                checksums.insert(exercise_id, checksum.to_string());
            }

            let update_result = core
                .get_exercise_updates(course_id, checksums)
                .context("Failed to get exercise updates")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::RetrievedData,
                percent_done: 1.0,
                data: Some(update_result),
            };
            print_output(&output)?;
        } else if let Some(_matches) = matches.subcommand_matches("mark-review-as-read") {
            let review_update_url = matches.value_of("review-update-url").unwrap();
            core.mark_review_as_read(review_update_url.to_string())
                .context("Failed to mark review as read")?;
        } else if let Some(matches) = matches.subcommand_matches("get-unread-reviews") {
            let reviews_url = matches.value_of("reviews-url").unwrap();
            let reviews_url = into_url(reviews_url)?;

            let reviews = core
                .get_unread_reviews(reviews_url)
                .context("Failed to get unread reviews")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::LoggedOut,
                percent_done: 1.0,
                data: Some(reviews),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("request-code-review") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let message_for_reviewer = matches.value_of("message-for-reviewer").unwrap();

            let locale = matches.value_of("locale");
            let locale = if let Some(locale) = locale {
                Some(into_locale(locale)?)
            } else {
                None
            };

            let new_submission = core
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .context("Failed to request code review")?;

            let output = Output {
                status: Status::Successful,
                message: None,
                result: OutputResult::SentData,
                percent_done: 1.0,
                data: Some(new_submission),
            };
            print_output(&output)?;
        } else if let Some(matches) = matches.subcommand_matches("download-model-solution") {
            let solution_download_url = matches.value_of("solution-download-url").unwrap();
            let solution_download_url = into_url(solution_download_url)?;

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            core.download_model_solution(solution_download_url, target)
                .context("Failed to download model solution")?;
        } else if let Some(matches) = matches.subcommand_matches("reset-exercise") {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let save_old_state = matches.is_present("save-old-state");

            if save_old_state {
                let submission_url = matches.value_of("submission_url").unwrap();
                let submission_url = into_url(submission_url)?;
                core.submit(submission_url, exercise_path, None)?;
            }
            core.reset(exercise_id, exercise_path)?;
        } else if let Some(matches) = matches.subcommand_matches("download-old-submission") {
            let exercise_id = matches.value_of("exercise-id").unwrap();
            let exercise_id = into_usize(exercise_id)?;

            let submission_id = matches.value_of("submission-id").unwrap();
            let submission_id = into_usize(submission_id)?;

            let output_path = matches.value_of("output-path").unwrap();
            let output_path = Path::new(output_path);

            let save_old_state = matches.is_present("save-old-state");

            if save_old_state {
                let submission_url = matches.value_of("submission_url").unwrap();
                let submission_url = into_url(submission_url)?;
                core.submit(submission_url, output_path, None)?;
            }
            core.reset(exercise_id, output_path)?;

            let temp_zip = NamedTempFile::new().context("Failed to create a temporary archive")?;
            core.download_old_submission(submission_id, temp_zip.path())?;
            task_executor::extract_project(temp_zip.path(), output_path)?;
        }
    }
    Ok(())
}

fn print_output<T: Serialize + Debug>(output: &Output<T>) -> Result<()> {
    let result = serde_json::to_string(&output)
        .with_context(|| format!("Failed to convert {:?} to JSON", output))?;
    println!("{}", result);
    Ok(())
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

fn find_exercise_directories(exercise_path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    for entry in WalkDir::new(exercise_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(submission_processing::is_hidden_dir)
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s == "private")
                .unwrap_or(false)
        })
        .filter(submission_processing::contains_tmcignore)
    {
        // TODO: Java implementation doesn't scan root directories
        if task_executor::is_exercise_root_directory(entry.path()) {
            paths.push(entry.into_path())
        }
    }
    paths
}

fn run_checkstyle(exercise_path: &Path, output_path: &Path, locale: Language) -> Result<()> {
    let check_result =
        task_executor::run_check_code_style(exercise_path, locale).with_context(|| {
            format!(
                "Failed to check code style for project at {}",
                exercise_path.display()
            )
        })?;
    if let Some(check_result) = check_result {
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
    Ok(())
}
