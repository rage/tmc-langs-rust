//! CLI client for TMC

use clap::{App, Arg, Error, ErrorKind, SubCommand};
use isolang::Language;
use log::debug;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tmc_langs_core::TmcCore;
use tmc_langs_framework::io::submission_processing;
use tmc_langs_util::task_executor;
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
                .long("locale")
                .help("Language as a three letter ISO 639-3 code, e.g. 'eng' or 'fin'.")
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
            .about("UNIMPLEMENTED. Prepares from submission and solution project for which the tests can be run in sandbox.")
            .arg(Arg::with_name("clonePath")
                .long("clonePath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("submissionPath")
                .long("submissionPath")
                .required(true)
                .takes_value(true))
            .arg(Arg::with_name("outputPath")
                .long("outputPath")
                .required(true)
                .takes_value(true)))

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

                .subcommand(SubCommand::with_name("send-diagnostics")
                    .about("Send diagnostics."))

                .subcommand(SubCommand::with_name("download-or-update-exercises")
                    .about("Download exercise.")
                    .arg(Arg::with_name("exercises")
                        .help("A list of exercise ids and the paths where they should be extracted to, formatted as follows: \"123:path/to/exercise,234:another/path\". The paths should not contain the characters ':' or ',' as they are used as separators (TODO: rework)")
                        .long("exercises")
                        .required(true)
                        .takes_value(true)))

                .subcommand(SubCommand::with_name("get-course-details")
                    .about("Get course details.")
                    .arg(Arg::with_name("courseId")
                        .help("Course ID")
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
                    .about("Send exercise to pastebin with comment."))
                .subcommand(SubCommand::with_name("run-checkstyle")
                    .about("Run checkstyle."))
                .subcommand(SubCommand::with_name("run-tests")
                    .about("Run tests."))
                .subcommand(SubCommand::with_name("send-feedback")
                    .about("Send feedback."))
                .subcommand(SubCommand::with_name("send-snapshot-events")
                    .about("Send snapshot events(?)."))
                .subcommand(SubCommand::with_name("submit")
                    .about("Submit exercise."))
                .subcommand(SubCommand::with_name("get-exercise-updates")
                    .about("Get exercise updates."))
                .subcommand(SubCommand::with_name("mark-review-as-read")
                    .about("Mark review as read."))
                .subcommand(SubCommand::with_name("get-unread-reviews")
                    .about("Get unread reviews."))
                .subcommand(SubCommand::with_name("request-code-review")
                    .about("Request code review."))
                .subcommand(SubCommand::with_name("download-model-solution")
                    .about("Download model solutions.")))

        .get_matches();

    // non-core
    if let Some(matches) = matches.subcommand_matches("checkstyle") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let locale = matches.value_of("locale").unwrap();
        let locale = Language::from_639_3(&locale).unwrap_or_else(|| {
            Error::with_description(
                &format!("Invalid locale: {}", locale),
                ErrorKind::InvalidValue,
            )
            .exit()
        });

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
    } else if let Some(matches) = matches.subcommand_matches("prepare-submission") {
        let clone_path = matches.value_of("clonePath").unwrap();
        let __clone_path = Path::new(clone_path);

        let submission_path = matches.value_of("submissionPath").unwrap();
        let _submission_path = Path::new(submission_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let _output_path = Path::new(output_path);

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
        let locale = match locale {
            Some(locale) => {
                let iso_locale = Language::from_639_3(&locale);
                Some(iso_locale.unwrap_or_else(|| {
                    Error::with_description(
                        &format!("Invalid locale: {}", locale),
                        ErrorKind::InvalidValue,
                    )
                    .exit()
                }))
            }
            None => None,
        };

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

        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });

        serde_json::to_writer(output_file, &test_result).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write test results as JSON to {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });

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
                    "Exercise path's file name  '{:?}' was not valid UTF8",
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

        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });

        serde_json::to_writer(output_file, &scan_result).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write scan result as JSON to {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
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

        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });

        serde_json::to_writer(output_file, &exercises).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write exercises as JSON to {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
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

        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}: {}", output_path.display(), e),
                ErrorKind::Io,
            )
            .exit()
        });

        serde_json::to_writer(output_file, &config).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write exercise package config as JSON to {}: {}",
                    output_path.display(),
                    e
                ),
                ErrorKind::Io,
            )
            .exit()
        });
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
        let mut core = TmcCore::new_in_config("https://tmc.mooc.fi").unwrap_or_else(|e| {
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

        if let Some(matches) = matches.subcommand_matches("get-organizations") {
            let orgs = core.get_organizations().unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to get organizations: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            let orgs = serde_json::to_value(&orgs).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to convert organizations to JSON: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });
            println!("{}", orgs);
        } else if let Some(matches) = matches.subcommand_matches("send-diagnostics") {
            core.send_diagnostics()
        } else if let Some(matches) = matches.subcommand_matches("download-or-update-exercises") {
            let exercises = matches.value_of("exercises").unwrap();
            let exercises = exercises
                .split(',')
                .into_iter()
                .map(|e| {
                    let mut split = e.split(':');
                    let exercise_id = split.next().unwrap_or_else(|| {
                        Error::with_description(
                            "Malformed exercise list",
                            ErrorKind::ValueValidation,
                        )
                        .exit()
                    });

                    let path = split.next().unwrap_or_else(|| {
                        Error::with_description(
                            "Malformed exercise list",
                            ErrorKind::ValueValidation,
                        )
                        .exit()
                    });

                    let exercise_id = usize::from_str_radix(exercise_id, 10).unwrap_or_else(|e| {
                        Error::with_description(
                            &format!("Malformed exercise id {}: {}", exercise_id, e),
                            ErrorKind::ValueValidation,
                        )
                        .exit()
                    });

                    let path = Path::new(path);
                    (exercise_id, path)
                })
                .collect();

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
            let course_id = usize::from_str_radix(course_id, 10).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Malformed course id {}: {}", course_id, e),
                    ErrorKind::Io,
                )
                .exit()
            });
            let course_details = core.get_course_details(course_id).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to get course details: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            let course_details = serde_json::to_value(&course_details).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to convert course details to JSON: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            println!("{}", course_details);
        } else if let Some(matches) = matches.subcommand_matches("list-courses") {
            let organization_slug = matches.value_of("organization").unwrap();
            let courses = core.list_courses(organization_slug).unwrap_or_else(|e| {
                Error::with_description(&format!("Failed to get courses: {}", e), ErrorKind::Io)
                    .exit()
            });

            let courses = serde_json::to_string(&courses).unwrap_or_else(|e| {
                Error::with_description(
                    &format!("Failed to convert courses to JSON: {}", e),
                    ErrorKind::Io,
                )
                .exit()
            });

            println!("{}", courses);
        } else if let Some(matches) = matches.subcommand_matches("paste-with-comment") {
            //core.paste_with_comment()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("run-checkstyle") {
            //core.run_checkstyle()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("run-tests") {
            //core.run_tests()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("send-feedback") {
            //core.send_feedback()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("send-snapshot-events") {
            //core.send_snapshot_events()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("submit") {
            //core.submit()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("get-exercise-updates") {
            //core.get_exercise_updates()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("mark-review-as-read") {
            //core.mark_review_as_read()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("get-unread-reviews") {
            //core.get_unread_reviews()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("request-code-review") {
            //core.request_code_review()
            todo!()
        } else if let Some(matches) = matches.subcommand_matches("download-model-solution") {
            //core.download_model_solution()
            todo!()
        }
    }
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
                &format!("Failed to check code style at {}", exercise_path.display()),
                ErrorKind::Io,
            )
            .exit()
        });
    if let Some(check_result) = check_result {
        let output_file = File::create(output_path).unwrap_or_else(|e| {
            Error::with_description(
                &format!("Failed to create file at {}", output_path.display()),
                ErrorKind::Io,
            )
            .exit()
        });
        serde_json::to_writer(output_file, &check_result).unwrap_or_else(|e| {
            Error::with_description(
                &format!(
                    "Failed to write check results as JSON to {}",
                    output_path.display()
                ),
                ErrorKind::Io,
            )
            .exit()
        });
    }
}
