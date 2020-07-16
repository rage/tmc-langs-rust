//! CLI client for TMC

use anyhow::{Context, Result};
use clap::{App, Arg, Error, ErrorKind, SubCommand};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tmc_langs_core::oauth2::{
    basic::BasicTokenType, AccessToken, EmptyExtraTokenFields, Scope, StandardTokenResponse,
};
use tmc_langs_core::{FeedbackAnswer, TmcCore, Token};
use tmc_langs_framework::io::submission_processing;
use tmc_langs_util::{task_executor, Language};
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
        let error = serde_json::json! {
            {
                "error": {
                    "message": e.to_string(),
                    "causes": causes,
                }
            }
        };
        println!("{:#}", error);
        quit::with_code(1);
    }
}

fn run() -> Result<()> {
    let matches = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))

        .subcommand(SubCommand::with_name("checkstyle")
            .about("Run checkstyle or similar plugin to project if applicable.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("locale")
                .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                .long("locale")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("compress-project")
            .about("Compress target project into a ZIP.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("extract-project")
            .about("Given a downloaded zip, extracts to specified folder.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-solutions")
            .about("Prepare a presentable solution from the original.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-stubs")
            .about("Prepare a stub exercise from the original.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-submission")
            .about("UNIMPLEMENTED. Prepares from submission and solution project for which the tests can be run in sandbox."))

        .subcommand(SubCommand::with_name("run-tests")
            .about("Run the tests for the exercise.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("checkstyle-output-path")
                .long("checkstyle-output-path")
                .help("Runs checkstyle if defined")
                .takes_value(true))
            .arg(Arg::with_name("locale")
                .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                .long("locale")
                .help("Required if checkstyle-output-path is defined")
                .takes_value(true)))

        .subcommand(SubCommand::with_name("scan-exercise")
            .about("Produce an exercise description of an exercise directory.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("find-exercises")
            .about("Produce list of found exercises.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("get-exercise-packaging-configuration")
            .about("Returns configuration of under which folders student and nonstudent files are located.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("output-path")
                .long("output-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("clean")
            .about("Clean target directory.")
            .arg(Arg::with_name("exercise-path")
                .long("exercise-path")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("core")
            .about("tmc-core commands. The program will ask for your TMC password through stdin.")
            .arg(Arg::with_name("client-name")
                .help("Name used to differentiate between different TMC clients")
                .long("client-name")
                .required(true)
                .takes_value(true))

            .subcommand(SubCommand::with_name("login")
                .about("Login and store OAuth2 token in config.")
                .arg(Arg::with_name("email")
                    .help("The email address of your TMC account")
                    .long("email")
                    .takes_value(true))
                .arg(Arg::with_name("set-access-token")
                    .help("The OAUTH2 access token that should be used for authentication")
                    .long("set-access-token")
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("logout")
                .about("Logout and remove OAuth2 token from config."))

            .subcommand(SubCommand::with_name("logged-in")
                .about("Check if the CLI is logged in. Prints the access token if so."))

            .subcommand(SubCommand::with_name("get-organizations")
                .about("Get organizations."))

            .subcommand(SubCommand::with_name("download-or-update-exercises")
                .about("Download exercise.")
                .arg(Arg::with_name("exercise")
                    .help("An exercise. Takes two values, an exercise id and an exercise path. Multiple exercises can be given.")
                    .long("exercise")
                    .required(true)
                    .takes_value(true)
                    .number_of_values(2)
                    .value_names(&["exercise-id", "exercise-path"])
                    .multiple(true)))

            .subcommand(SubCommand::with_name("get-course-details")
                .about("Get course details.")
                .arg(Arg::with_name("course-id")
                    .long("course-id")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("list-courses")
                .about("List courses.")
                .arg(Arg::with_name("organization")
                    .help("Organization slug (e.g. mooc, hy).")
                    .long("organization")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("paste-with-comment")
                .about("Send exercise to pastebin with comment.")
                .arg(Arg::with_name("submission-url")
                    .long("submission-url")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submission-path")
                    .long("submission-path")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("paste-message")
                    .long("paste-message")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("run-checkstyle")
                .about("Run checkstyle.")
                .arg(Arg::with_name("exercise-path")
                    .long("exercise-path")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("run-tests")
                .about("Run tests.")
                .arg(Arg::with_name("exercise-path")
                    .long("exercise-path")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("send-feedback")
                .about("Send feedback.")
                .arg(Arg::with_name("feedback-url")
                    .long("feedback-url")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("feedback")
                    .help("A feedback answer. Takes two values, a feedback answer id and the answer. Multiple feedback arguments can be given.")
                    .long("feedback")
                    .required(true)
                    .takes_value(true)
                    .number_of_values(2)
                    .value_names(&["feedback-answer-id", "answer"])
                    .multiple(true)))

            .subcommand(SubCommand::with_name("submit")
                .about("Submit exercise.")
                .arg(Arg::with_name("submission-url")
                    .long("submission-url")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submission-path")
                    .long("submission-path")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")                    
                    .long("locale")
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("wait-for-submission")
                .about("Wait for a submission to finish.")
                .arg(Arg::with_name("submission-url")
                    .long("submission-url")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("get-exercise-updates")
                .about("Get exercise updates.")
                .arg(Arg::with_name("course-id")
                    .long("course-id")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("exercise")
                    .help("An exercise. Takes two values, an exercise id and a checksum. Multiple exercises can be given.")
                    .long("exercise")
                    .required(true)
                    .takes_value(true)
                    .number_of_values(2)
                    .value_names(&["exercise-id", "checksum"])
                    .multiple(true)))

            .subcommand(SubCommand::with_name("mark-review-as-read")
                .about("Mark review as read.")
                .arg(Arg::with_name("review-update-url")
                    .long("review-update-url")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("get-unread-reviews")
                .about("Get unread reviews.")
                .arg(Arg::with_name("reviews-url")
                    .long("reviews-url")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("request-code-review")
                .about("Request code review.")
                .arg(Arg::with_name("submission-url")
                    .long("submission-url")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submission-path")
                    .long("submission-path")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("message-for-reviewer")
                    .long("message-for-reviewer")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("download-model-solution")
                .about("Download model solutions.")
                .arg(Arg::with_name("solution-download-url")
                    .long("solution-download-url")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("target")
                    .long("target")
                    .required(true)
                    .takes_value(true))))

        .get_matches();

    // non-core
    if let Some(matches) = matches.subcommand_matches("checkstyle") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let locale = matches.value_of("locale").unwrap();
        let locale = into_locale(locale)?;

        run_checkstyle(exercise_path, output_path, locale)?
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
    } else if let Some(matches) = matches.subcommand_matches("extract-project") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        task_executor::extract_project(exercise_path, output_path)
            .with_context(|| format!("Failed to extract project at {}", output_path.display()))?;
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
    } else if let Some(_matches) = matches.subcommand_matches("prepare-submission") {
        Error::with_description(
            "This command is unimplemented.",
            ErrorKind::InvalidSubcommand,
        )
        .exit();
    } else if let Some(matches) = matches.subcommand_matches("run-tests") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("output-path").unwrap();
        let output_path = Path::new(output_path);

        let checkstyle_output_path = matches.value_of("checkstyle-output-path");
        let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

        let optional_locale = matches.value_of("locale");
        let optional_locale = optional_locale.map(into_locale);

        let test_result = task_executor::run_tests(exercise_path).with_context(|| {
            format!(
                "Failed to run tests for exercise at {}",
                exercise_path.display()
            )
        })?;

        write_result_to_file_as_json(&test_result, output_path)?;

        if let Some(checkstyle_output_path) = checkstyle_output_path {
            let locale = optional_locale.unwrap_or_else(|| {
                Error::with_description(
                    "Locale must be given if checkstyle-output-path is given.",
                    ErrorKind::ArgumentNotFound,
                )
                .exit()
            })?;

            run_checkstyle(exercise_path, checkstyle_output_path, locale)?;
        }
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
    } else if let Some(matches) = matches.subcommand_matches("clean") {
        let exercise_path = matches.value_of("exercise-path").unwrap();
        let exercise_path = Path::new(exercise_path);

        task_executor::clean(exercise_path)
            .with_context(|| format!("Failed to clean exercise at {}", exercise_path.display(),))?;
    }

    // core
    if let Some(matches) = matches.subcommand_matches("core") {
        let root_url =
            env::var("TMC_CORE_CLI_ROOT_URL").unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
        let mut core = TmcCore::new_in_config(root_url)
            .with_context(|| format!("Failed to create TmcCore"))?;
        // set progress report to print the updates to stdout as JSON
        core.set_progress_report(|update| println!("{}", serde_json::to_string(&update).unwrap()));

        // set token if a credentials.json is found for the client name
        let client_name = matches.value_of("client-name").unwrap();
        let tmc_dir = format!("tmc-{}", client_name);

        let config_dir = match env::var("TMC_LANGS_CLI_CONFIG_DIR") {
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
                let token = core
                    .authenticate(client_name, email.to_string(), password)
                    .context("Failed to authenticate with TMC")?;
                token
            } else {
                Error::with_description(
                    "Either the --email or --set-access-token argument should be given",
                    ErrorKind::MissingRequiredArgument,
                )
                .exit();
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
        } else if let Some(_matches) = matches.subcommand_matches("logout") {
            if credentials_path.exists() {
                fs::remove_file(&credentials_path).with_context(|| {
                    format!(
                        "Failed to remove credentials at {}",
                        credentials_path.display()
                    )
                })?;
            }
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
                println!("{}", serde_json::to_string(&token).unwrap());
            } else {
                println!(
                    "{}",
                    serde_json::json! {
                        {
                            "message": "Not logged in."
                        }
                    }
                )
            }
        } else if let Some(_matches) = matches.subcommand_matches("get-organizations") {
            let orgs = core
                .get_organizations()
                .context("Failed to get organizations")?;

            print_result_as_json(&orgs)?;
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
        } else if let Some(matches) = matches.subcommand_matches("get-course-details") {
            let course_id = matches.value_of("course-id").unwrap();
            let course_id = into_usize(course_id)?;

            let course_details = core
                .get_course_details(course_id)
                .context("Failed to get course details")?;

            print_result_as_json(&course_details)?;
        } else if let Some(matches) = matches.subcommand_matches("list-courses") {
            let organization_slug = matches.value_of("organization").unwrap();
            let courses = core
                .list_courses(organization_slug)
                .context("Failed to get courses")?;

            print_result_as_json(&courses)?;
        } else if let Some(matches) = matches.subcommand_matches("paste-with-comment") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);
            let paste_message = matches.value_of("paste-message").unwrap();

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let new_submission = core
                .paste_with_comment(
                    submission_url,
                    submission_path,
                    paste_message.to_string(),
                    locale,
                )
                .context("Failed to get paste with comment")?;

            print_result_as_json(&new_submission)?;
        } else if let Some(matches) = matches.subcommand_matches("run-checkstyle") {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);
            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let validation_result = core
                .run_checkstyle(exercise_path, locale)
                .context("Failed to run checkstyle")?;

            print_result_as_json(&validation_result)?;
        } else if let Some(matches) = matches.subcommand_matches("run-tests") {
            let exercise_path = matches.value_of("exercise-path").unwrap();
            let exercise_path = Path::new(exercise_path);

            let run_result = core
                .run_tests(exercise_path)
                .context("Failed to run tests")?;

            print_result_as_json(&run_result)?;
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

            print_result_as_json(&response)?;
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

            let new_submission = core
                .submit(submission_url, submission_path, optional_locale)
                .context("Failed to submit")?;

            print_result_as_json(&new_submission)?;
        } else if let Some(matches) = matches.subcommand_matches("wait-for-submission") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_finished = core
                .wait_for_submission(submission_url)
                .context("Failed while waiting for submissions")?;
            let submission_finished = serde_json::to_string(&submission_finished)
                .context("Failed to serialize submission results")?;
            println!("{}", submission_finished);
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

            print_result_as_json(&update_result)?;
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

            print_result_as_json(&reviews)?;
        } else if let Some(matches) = matches.subcommand_matches("request-code-review") {
            let submission_url = matches.value_of("submission-url").unwrap();
            let submission_url = into_url(submission_url)?;

            let submission_path = matches.value_of("submission-path").unwrap();
            let submission_path = Path::new(submission_path);

            let message_for_reviewer = matches.value_of("message-for-reviewer").unwrap();

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale)?;

            let new_submission = core
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .context("Failed to request code review")?;

            print_result_as_json(&new_submission)?;
        } else if let Some(matches) = matches.subcommand_matches("download-model-solution") {
            let solution_download_url = matches.value_of("solution-download-url").unwrap();
            let solution_download_url = into_url(solution_download_url)?;

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            core.download_model_solution(solution_download_url, target)
                .context("Failed to download model solution")?;
        }
    }
    Ok(())
}

fn print_result_as_json<T: Serialize>(result: &T) -> Result<()> {
    let result = serde_json::to_string(&result).context("Failed to convert result to JSON")?;
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
        .or(Language::from_639_1(arg))
        .or(Language::from_639_3(arg))
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
