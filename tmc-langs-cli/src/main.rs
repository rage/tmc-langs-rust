//! CLI client for TMC

use clap::{App, Arg, Error, ErrorKind, SubCommand};
use log::debug;
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tmc_langs_core::{FeedbackAnswer, TmcCore};
use tmc_langs_framework::io::submission_processing;
use tmc_langs_util::{task_executor, Language};
use url::Url;
use walkdir::WalkDir;

fn main() {
    env_logger::init();

    let matches = App::new("TestMyCode")
        .version("0.1.0")
        .author("Daniel Martinez <daniel.x.martinez@helsinki.fi")
        .about("CLI client for TMC")

        .subcommand(SubCommand::with_name("checkstyle")
            .about("Run checkstyle or similar plugin to project if applicable.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("locale")
                .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                .long("locale")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("compress-project")
            .about("Compress target project into a ZIP.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("extract-project")
            .about("Given a downloaded zip, extracts to specified folder.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-solutions")
            .about("Prepare a presentable solution from the original.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-stubs")
            .about("Prepare a stub exercise from the original.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("prepare-submission")
            .about("UNIMPLEMENTED. Prepares from submission and solution project for which the tests can be run in sandbox."))

        .subcommand(SubCommand::with_name("run-tests")
            .about("Run the tests for the exercise.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("checkstyleOutputPath")
                .long("checkstyleOutputPath")
                .help("Runs checkstyle if defined")
                .takes_value(true)))
            .arg(Arg::with_name("locale")
                .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                .long("locale")
                .help("Required if checkstyleOutputPath is defined")
                .takes_value(true))

        .subcommand(SubCommand::with_name("scan-exercise")
            .about("Produce an exercise description of an exercise directory.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("find-exercises")
            .about("Produce list of found exercises.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("get-exercise-packaging-configuration")
            .about("Returns configuration of under which folders student and nonstudent files are located.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("clean")
            .about("Clean target directory.")
            .arg(Arg::with_name("exercisePath")
                .long("exercisePath")
                .required(true)
                .takes_value(true)))

        .subcommand(SubCommand::with_name("core")
            .about("tmc-core commands. The program will ask for your TMC password through stdin.")
            .arg(Arg::with_name("email")
                .help("The email associated with your TMC account.")
                .long("email")
                .required(true)
                .takes_value(true))

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
                    .multiple(true)))

            .subcommand(SubCommand::with_name("get-course-details")
                .about("Get course details.")
                .arg(Arg::with_name("courseId")
                    .long("courseId")
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
                .arg(Arg::with_name("submissionUrl")
                    .long("submissionUrl")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submissionPath")
                    .long("submissionPath")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("pasteMessage")
                    .long("pasteMessage")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("run-checkstyle")
                .about("Run checkstyle.")
                .arg(Arg::with_name("exercisePath")
                    .long("exercisePath")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("run-tests")
                .about("Run tests.")
                .arg(Arg::with_name("exercisePath")
                    .long("exercisePath")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("send-feedback")
                .about("Send feedback.")
                .arg(Arg::with_name("feedbackUrl")
                    .long("feedbackUrl")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("feedback")
                    .help("A feedback answer. Takes two values, a feedback answer id and the answer. Multiple feedback arguments can be given.")
                    .long("feedback")
                    .required(true)
                    .takes_value(true)
                    .number_of_values(2)
                    .multiple(true)))

            .subcommand(SubCommand::with_name("submit")
                .about("Submit exercise.")
                .arg(Arg::with_name("submissionUrl")
                    .long("submissionUrl")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submissionPath")
                    .long("submissionPath")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")                    
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("get-exercise-updates")
                .about("Get exercise updates.")
                .arg(Arg::with_name("courseId")
                    .long("courseId")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("exercise")
                    .help("An exercise. Takes two values, an exercise id and a checksum. Multiple exercises can be given.")
                    .long("exercise")
                    .required(true)
                    .takes_value(true)
                    .number_of_values(2)
                    .multiple(true)))

            .subcommand(SubCommand::with_name("mark-review-as-read")
                .about("Mark review as read.")
                .arg(Arg::with_name("reviewUpdateUrl")
                    .long("reviewUpdateUrl")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("get-unread-reviews")
                .about("Get unread reviews.")
                .arg(Arg::with_name("reviewsUrl")
                    .long("reviewsUrl")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("request-code-review")
                .about("Request code review.")
                .arg(Arg::with_name("submissionUrl")
                    .long("submissionUrl")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("submissionPath")
                    .long("submissionPath")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("messageForReviewer")
                    .long("messageForReviewer")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("locale")
                    .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
                    .long("locale")
                    .required(true)
                    .takes_value(true)))

            .subcommand(SubCommand::with_name("download-model-solution")
                .about("Download model solutions.")
                .arg(Arg::with_name("solutionDownloadUrl")
                    .long("solutionDownloadUrl")
                    .required(true)
                    .takes_value(true))
                .arg(Arg::with_name("target")
                    .long("target")
                    .required(true)
                    .takes_value(true))))

        .get_matches();

    // non-core
    if let Some(matches) = matches.subcommand_matches("checkstyle") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let locale = matches.value_of("locale").unwrap();
        let locale = into_locale(locale);

        run_checkstyle(exercise_path, output_path, locale)
    } else if let Some(matches) = matches.subcommand_matches("compress-project") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let data = task_executor::compress_project(exercise_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to compress project at {}: {}",
                    exercise_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });

        let mut output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });

        output_file.write_all(&data).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to write to {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });
    } else if let Some(matches) = matches.subcommand_matches("extract-project") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        task_executor::extract_project(exercise_path, output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to extract project at {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    } else if let Some(matches) = matches.subcommand_matches("prepare-solutions") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        task_executor::prepare_solutions(&[exercise_path.to_path_buf()], output_path)
            .unwrap_or_else(|e| {
                Error::with_description(
                    &format!(
                        "Failed to prepare solutions for exercise {}: {}",
                        exercise_path.display(),
                        e
                    ),
                    ErrorKind::Io,
                )
                .exit()
            });
    } else if let Some(matches) = matches.subcommand_matches("prepare-stubs") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let exercises = find_exercise_directories(exercise_path);

        task_executor::prepare_stubs(exercises, exercise_path, output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to prepare stubs for exercise {}: {}",
                    exercise_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    } else if let Some(_matches) = matches.subcommand_matches("prepare-submission") {
        Error::with_description(
            "This command is unimplemented.",
            ErrorKind::InvalidSubcommand,
        )
        .exit();
    } else if let Some(matches) = matches.subcommand_matches("run-tests") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let checkstyle_output_path = matches.value_of("checkstyleOutputPath");
        let checkstyle_output_path: Option<&Path> = checkstyle_output_path.map(Path::new);

        let locale = matches.value_of("locale");
        let locale = locale.map(into_locale);

        let test_result = task_executor::run_tests(exercise_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to run tests for exercise {}: {}",
                    exercise_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });

        write_result_to_file_as_json(&test_result, output_path);

        if let Some(checkstyle_output_path) = checkstyle_output_path {
            let locale = locale.unwrap_or_else(|| {
                Error::with_description(
                    "Locale must be given if checkstyleOutputPath is given.",
                    ErrorKind::ArgumentNotFound,
                )
                .exit()
            });

            run_checkstyle(exercise_path, checkstyle_output_path, locale);
        }
    } else if let Some(matches) = matches.subcommand_matches("scan-exercise") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let exercise_name = exercise_path.file_name().unwrap_or_else(|| {
            Error::with_description(
                &format!(
                    "No file name found in exercise path {}",
                    exercise_path.display()
                ),
                ErrorKind::ValueValidation,
            )
            .exit()
        });

        let exercise_name = exercise_name.to_str().unwrap_or_else(|| {
            Error::with_description(
                &format!(
                    "Exercise path's file name '{:?}' was not valid UTF8",
                    exercise_name
                ),
                ErrorKind::InvalidUtf8,
            )
            .exit()
        });

        let scan_result = task_executor::scan_exercise(exercise_path, exercise_name.to_string())
            .unwrap_or_else(|e| {
                Error::with_description(
                    &format!(
                        "Failed to scan exercise at {}: {}",
                        exercise_path.display(),
                        e
                    ),
                    ErrorKind::Io,
                )
                .exit()
            });

        write_result_to_file_as_json(&scan_result, output_path);
    } else if let Some(matches) = matches.subcommand_matches("find-exercises") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let mut exercises = vec![];
        for entry in WalkDir::new(exercise_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() == "private")
            .filter(submission_processing::is_hidden_dir)
            .filter(submission_processing::contains_tmcignore)
        {
            debug!("processing {}", entry.path().display());
            // TODO: Java implementation doesn't scan root directories
            if task_executor::is_exercise_root_directory(entry.path()) {
                exercises.push(entry.into_path());
            }
        }

        write_result_to_file_as_json(&exercises, output_path);
    } else if let Some(matches) = matches.subcommand_matches("get-exercise-packaging-configuration")
    {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let config = task_executor::get_exercise_packaging_configuration(exercise_path)
            .unwrap_or_else(|e| {
                Error::with_description(
                    &format!(
                        "Failed to get exercise packaging configuration for exercise {}: {}",
                        exercise_path.display(),
                        e
                    ),
                    ErrorKind::Io,
                )
                .exit()
            });

        write_result_to_file_as_json(&config, output_path);
    } else if let Some(matches) = matches.subcommand_matches("clean") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        task_executor::clean(exercise_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to clean exercise at {}: {}",
                    exercise_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    }

    // core
    if let Some(matches) = matches.subcommand_matches("core") {
        let root_url =
            env::var("TMC_CORE_CLI_ROOT_URL").unwrap_or_else(|_| "https://tmc.mooc.fi".to_string());
        let mut core = TmcCore::new_in_config(root_url).unwrap_or_else(|e| {
            Error::with_description(&format!("Failed to create TmcCore: {}", e), ErrorKind::Io)
                .exit()
        });

        let email = matches.value_of("email").unwrap();
        // TODO: "Please enter password" and quiet param
        let password = rpassword::read_password().unwrap_or_else(|e| {
            Error::with_description(&format!("Failed to read password: {}", e), ErrorKind::Io)
                .exit()
        });

        core.authenticate("vscode_plugin", email.to_string(), password)
            .unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to authenticate with TMC: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

        if let Some(_matches) = matches.subcommand_matches("get-organizations") {
            let orgs = core.get_organizations().unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to get organizations: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            print_result_as_json(&orgs);
        } else if let Some(matches) = matches.subcommand_matches("download-or-update-exercises") {
            let mut exercise_args = matches.values_of("exercise").unwrap();
            let mut exercises = vec![];
            while let Some(exercise_id) = exercise_args.next() {
                let exercise_id = into_usize(exercise_id);
                let exercise_path = exercise_args.next().unwrap(); // safe unwrap because each --exercise takes 2 arguments
                let exercise_path = Path::new(exercise_path);
                exercises.push((exercise_id, exercise_path));
            }

            core.download_or_update_exercises(exercises)
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to download exercises: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });
        } else if let Some(matches) = matches.subcommand_matches("get-course-details") {
            let course_id = matches.value_of("courseId").unwrap();
            let course_id = into_usize(course_id);

            let course_details = core.get_course_details(course_id).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to get course details: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            print_result_as_json(&course_details);
        } else if let Some(matches) = matches.subcommand_matches("list-courses") {
            let organization_slug = matches.value_of("organization").unwrap();
            let courses = core.list_courses(organization_slug).unwrap_or_else(|e| {
                Error::with_description(&format!("Failed to get courses: {}", e), ErrorKind::Io)
                    .exit()
            });

            print_result_as_json(&courses);
        } else if let Some(matches) = matches.subcommand_matches("paste-with-comment") {
            let submission_url = matches.value_of("submissionUrl").unwrap();
            let submission_url = into_url(submission_url);

            let submission_path = matches.value_of("submissionPath").unwrap();
            let submission_path = Path::new(submission_path);
            let paste_message = matches.value_of("pasteMessage").unwrap();

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale);

            let new_submission = core
                .paste_with_comment(
                    submission_url,
                    submission_path,
                    paste_message.to_string(),
                    locale,
                )
                .unwrap_or_else(|e| {
                    Error::with_description(&format!("Failed to get courses: {}", e), ErrorKind::Io)
                        .exit()
                });

            print_result_as_json(&new_submission);
        } else if let Some(matches) = matches.subcommand_matches("run-checkstyle") {
            let exercise_path = matches.value_of("exercisePath").unwrap();
            let exercise_path = Path::new(exercise_path);
            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale);

            let validation_result =
                core.run_checkstyle(exercise_path, locale)
                    .unwrap_or_else(|e| {
                        Error::with_description(
                            &format!("Failed to run checkstyle: {}", e),
                            ErrorKind::Io,
                        )
                        .exit()
                    });

            print_result_as_json(&validation_result);
        } else if let Some(matches) = matches.subcommand_matches("run-tests") {
            let exercise_path = matches.value_of("exercisePath").unwrap();
            let exercise_path = Path::new(exercise_path);

            let run_result = core.run_tests(exercise_path).unwrap_or_else(|e| {
                Error::with_description(&format!("Failed to run checkstyle: {}", e), ErrorKind::Io)
                    .exit()
            });

            print_result_as_json(&run_result);
        } else if let Some(matches) = matches.subcommand_matches("send-feedback") {
            let feedback_url = matches.value_of("feedbackUrl").unwrap();
            let feedback_url = into_url(feedback_url);

            let mut feedback_answers = matches.values_of("feedback").unwrap();
            let mut feedback = vec![];
            while let Some(feedback_id) = feedback_answers.next() {
                let question_id = into_usize(feedback_id);
                let answer = feedback_answers.next().unwrap().to_string(); // safe unwrap because --feedback always takes 2 values
                feedback.push(FeedbackAnswer {
                    question_id,
                    answer,
                });
            }

            let response = core
                .send_feedback(feedback_url, feedback)
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to send feedback: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });

            print_result_as_json(&response);
        } else if let Some(matches) = matches.subcommand_matches("submit") {
            let submission_url = matches.value_of("submissionUrl").unwrap();
            let submission_url = into_url(submission_url);

            let submission_path = matches.value_of("submissionPath").unwrap();
            let submission_path = Path::new(submission_path);

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale);

            let new_submission = core
                .submit(submission_url, submission_path, locale)
                .unwrap_or_else(|e| {
                    Error::with_description(&format!("Failed to submit: {}", e), ErrorKind::Io)
                        .exit()
                });

            print_result_as_json(&new_submission);
        } else if let Some(_matches) = matches.subcommand_matches("get-exercise-updates") {
            let course_id = matches.value_of("courseId").unwrap();
            let course_id = into_usize(course_id);

            let mut exercise_checksums = matches.values_of("exercise").unwrap();
            let mut checksums = HashMap::new();
            while let Some(exercise_id) = exercise_checksums.next() {
                let exercise_id = into_usize(exercise_id);
                let checksum = exercise_checksums.next().unwrap();
                checksums.insert(exercise_id, checksum.to_string());
            }

            let update_result = core
                .get_exercise_updates(course_id, checksums)
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to get exercise updates: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });

            print_result_as_json(&update_result);
        } else if let Some(_matches) = matches.subcommand_matches("mark-review-as-read") {
            let review_update_url = matches.value_of("reviewUpdateUrl").unwrap();
            core.mark_review_as_read(review_update_url.to_string())
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to mark review as read: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });
        } else if let Some(matches) = matches.subcommand_matches("get-unread-reviews") {
            let reviews_url = matches.value_of("reviewsUrl").unwrap();
            let reviews_url = into_url(reviews_url);

            let reviews = core.get_unread_reviews(reviews_url).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to get unread reviews: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            print_result_as_json(&reviews);
        } else if let Some(matches) = matches.subcommand_matches("request-code-review") {
            let submission_url = matches.value_of("submissionUrl").unwrap();
            let submission_url = into_url(submission_url);

            let submission_path = matches.value_of("submissionPath").unwrap();
            let submission_path = Path::new(submission_path);

            let message_for_reviewer = matches.value_of("messageForReviewer").unwrap();

            let locale = matches.value_of("locale").unwrap();
            let locale = into_locale(locale);

            let new_submission = core
                .request_code_review(
                    submission_url,
                    submission_path,
                    message_for_reviewer.to_string(),
                    locale,
                )
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to get unread reviews: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });

            print_result_as_json(&new_submission);
        } else if let Some(matches) = matches.subcommand_matches("download-model-solution") {
            let solution_download_url = matches.value_of("solutionDownloadUrl").unwrap();
            let solution_download_url = into_url(solution_download_url);

            let target = matches.value_of("target").unwrap();
            let target = Path::new(target);

            core.download_model_solution(solution_download_url, target)
                .unwrap_or_else(|e| {
                    Error::with_description(
                        &format!("Failed to download model solution: {}", e),
                        ErrorKind::Io,
                    )
                    .exit()
                });
        }
    }
}

fn print_result_as_json<T: Serialize>(result: &T) {
    let result = serde_json::to_string(&result).unwrap_or_else(|e| {
        Error::with_description(
            &format!("Failed to convert result to JSON: {}", e),
            ErrorKind::Io,
        )
        .exit()
    });

    println!("{}", result);
}

fn write_result_to_file_as_json<T: Serialize>(result: &T, output_path: &Path) {
    let output_file = File::create(output_path).unwrap_or_else(|e| {
        Error::with_description(
            &format!("Failed to create file at {}: {}", output_path.display(), e),
            ErrorKind::Io,
        )
        .exit()
    });

    serde_json::to_writer(output_file, result).unwrap_or_else(|e| {
        Error::with_description(
            &format!(
                "Failed to write result as JSON to {}: {}",
                output_path.display(),
                e
            ),
            ErrorKind::Io,
        )
        .exit()
    });
}

fn into_usize(arg: &str) -> usize {
    usize::from_str_radix(arg, 10).unwrap_or_else(|e| {
        Error::with_description(
            &format!(
                "Failed to convert argument to a non-negative integer {}: {}",
                arg, e
            ),
            ErrorKind::Io,
        )
        .exit()
    })
}

fn into_locale(arg: &str) -> Language {
    Language::from_639_3(arg).unwrap_or_else(|| {
        Error::with_description(&format!("Invalid locale: {}", arg), ErrorKind::InvalidValue).exit()
    })
}

fn into_url(arg: &str) -> Url {
    Url::parse(arg).unwrap_or_else(|e| {
        Error::with_description(&format!("Failed to url {}: {}", arg, e), ErrorKind::Io).exit()
    })
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

fn run_checkstyle(exercise_path: &Path, output_path: &Path, locale: Language) {
    let check_result =
        task_executor::run_check_code_style(exercise_path, locale).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to check code style at {}: {}",
                    exercise_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    if let Some(check_result) = check_result {
        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e,),
                ErrorKind::Io,
            )
            .exit()
        });
        serde_json::to_writer(output_file, &check_result).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write check results as JSON to {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    }
}
