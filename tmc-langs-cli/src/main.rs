//! CLI client for TMC

use anyhow::{Context, Result};
use clap::{App, Arg, SubCommand};
use isolang::Language;
use log::debug;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tmc_langs_core::TmcCore;
use tmc_langs_framework::io::submission_processing;
use tmc_langs_util::task_executor;
use walkdir::WalkDir;

#[derive(Error, Debug)]
enum CliError {
    #[error("Invalid locale {0}")]
    InvalidLocale(String),

    #[error("No file name in {0}")]
    NoFileName(PathBuf),

    #[error("{0:?} is not valid UTF-8")]
    InvalidUTF8(OsString),
}

fn main() -> Result<()> {
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
            .about("Prepares from submission and solution project for which the tests can be run in sandbox.")
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
        let locale = Language::from_639_3(&locale)
            .ok_or_else(|| CliError::InvalidLocale(locale.to_string()))?;

        run_checkstyle(exercise_path, output_path, locale)?;
    } else if let Some(matches) = matches.subcommand_matches("compress-project") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let data = task_executor::compress_project(exercise_path).with_context(|| {
            format!("Failed to compress project at {}", exercise_path.display())
        })?;

        let mut output_file = File::create(output_path)
            .with_context(|| format!("Failed to open {}", output_path.display()))?;

        output_file
            .write_all(&data)
            .with_context(|| format!("Failed to write to {}", output_path.display()))?;
    } else if let Some(matches) = matches.subcommand_matches("extract-project") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        task_executor::extract_project(exercise_path, output_path)?;
    } else if let Some(matches) = matches.subcommand_matches("prepare-solutions") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        task_executor::prepare_solutions(&[exercise_path.to_path_buf()], output_path)?;
    } else if let Some(matches) = matches.subcommand_matches("prepare-stubs") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let exercises = find_exercise_directories(exercise_path);

        task_executor::prepare_stubs(exercises, exercise_path, output_path)?;
    } else if let Some(matches) = matches.subcommand_matches("prepare-submission") {
        let clone_path = matches.value_of("clonePath").unwrap();
        let __clone_path = Path::new(clone_path);

        let submission_path = matches.value_of("submissionPath").unwrap();
        let _submission_path = Path::new(submission_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let _output_path = Path::new(output_path);

        unimplemented!("not implemented in the Java CLI")
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
                Some(iso_locale.ok_or_else(|| CliError::InvalidLocale(locale.to_string()))?)
            }
            None => None,
        };

        let test_result = task_executor::run_tests(exercise_path).context("Failed to run tests")?;

        let output_file = File::create(output_path)
            .with_context(|| format!("Failed to create {}", output_path.display()))?;

        serde_json::to_writer(output_file, &test_result)
            .with_context(|| format!("Failed to write JSON to {}", output_path.display()))?;

        if let Some(checkstyle_output_path) = checkstyle_output_path {
            let locale =
                locale.with_context(|| "Locale is required when checkstyleOutputPath is set")?;

            run_checkstyle(exercise_path, checkstyle_output_path, locale)
                .context("Failed to run checkstyle")?;
        }
    } else if let Some(matches) = matches.subcommand_matches("scan-exercise") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let exercise_name = exercise_path
            .file_name()
            .ok_or_else(|| CliError::NoFileName(exercise_path.to_path_buf()))?;
        let exercise_name = exercise_name
            .to_str()
            .ok_or_else(|| CliError::InvalidUTF8(exercise_name.to_os_string()))?;

        let scan_result = task_executor::scan_exercise(exercise_path, exercise_name.to_string())?;

        let output_file = File::create(output_path)
            .with_context(|| format!("Failed to create file at {}", output_path.display()))?;

        serde_json::to_writer(output_file, &scan_result)
            .with_context(|| format!("Failed to write JSON to {}", output_path.display()))?;
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
        let output_file = File::create(output_path)
            .with_context(|| format!("Failed to create file at {}", output_path.display()))?;
        serde_json::to_writer(output_file, &exercises)
            .with_context(|| format!("Failed to write JSON to {}", output_path.display()))?;
    } else if let Some(matches) = matches.subcommand_matches("get-exercise-packaging-configuration")
    {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        let output_path = matches.value_of("outputPath").unwrap();
        let output_path = Path::new(output_path);

        let config = task_executor::get_exercise_packaging_configuration(exercise_path)?;

        let output_file = File::create(output_path)
            .with_context(|| format!("Failed to create file at {}", output_path.display()))?;

        serde_json::to_writer(output_file, &config)
            .with_context(|| format!("Failed to write JSON to {}", output_path.display()))?;
    } else if let Some(matches) = matches.subcommand_matches("clean") {
        let exercise_path = matches.value_of("exercisePath").unwrap();
        let exercise_path = Path::new(exercise_path);

        task_executor::clean(exercise_path)?;
    }

    // core
    if let Some(matches) = matches.subcommand_matches("core") {
        let mut core =
            TmcCore::new_in_config("https://tmc.mooc.fi").expect("failed to create TmcCore");

        let email = matches.value_of("email").unwrap();
        // TODO: "Please enter password" and quiet param
        let password = rpassword::read_password().expect("failed to read password");
        core.authenticate("vscode_plugin", email.to_string(), password)
            .expect("failed to authenticate TmcCore");

        if let Some(matches) = matches.subcommand_matches("get-organizations") {
            let orgs = core
                .get_organizations()
                .expect("failed to fetch organizations");
            let orgs = serde_json::to_value(&orgs).expect("failed to parse to JSON");
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
                    let exercise_id = split.next().expect("malformed exercises");
                    let path = split.next().expect("malformed exercises");

                    let exercise_id =
                        usize::from_str_radix(exercise_id, 10).expect("malformed exercise id");
                    let path = Path::new(path);
                    (exercise_id, path)
                })
                .collect();

            core.download_or_update_exercises(exercises).unwrap()
        } else if let Some(matches) = matches.subcommand_matches("get-course-details") {
            let course_id = matches.value_of("courseId").unwrap();
            let course_id = usize::from_str_radix(course_id, 10).expect("malformed course id");
            let course_details = core
                .get_course_details(course_id)
                .expect("failed to get course details");
            let course_details =
                serde_json::to_value(&course_details).expect("failed to parse to JSON");
            println!("{}", course_details);
        } else if let Some(matches) = matches.subcommand_matches("list-courses") {
            let organization_slug = matches.value_of("organization").unwrap();
            let courses = core
                .list_courses(organization_slug)
                .expect("failed to get courses");
            let courses = serde_json::to_string(&courses).expect("failed to parse to JSON");
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

    Ok(())
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
    let check_result = task_executor::run_check_code_style(exercise_path, locale)?;
    if let Some(check_result) = check_result {
        let output_file = File::create(output_path)
            .with_context(|| format!("Failed to create file at {}", output_path.display()))?;
        serde_json::to_writer(output_file, &check_result)
            .with_context(|| format!("Failed to write JSON to {}", output_path.display()))?;
    }
    Ok(())
}
