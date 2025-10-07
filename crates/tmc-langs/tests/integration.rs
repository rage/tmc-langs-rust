use std::{
    fs::{File, OpenOptions},
    io::Seek,
    path::Path,
};
use tempfile::{NamedTempFile, TempDir};
use tmc_langs::{Compression, LangsError, RunStatus, file_util};
use tmc_testmycode_client::TestMyCodeClient;

#[test]
#[ignore = "\
Does a lot of work and requires admin credentials to access the solutions.\
Should only be ran to verify bigger changes that may break some exercises.\
It's recommended to save the log with something like cargo test | tee.
"]
fn test_policies_on_course_exercises() {
    env_logger::init();
    let courses = &[
        600,  // mooc-java-programming-i
        625,  // mooc-java-programming-ii
        1040, // mooc-working-with-text
        1489, // mooc-ohjelmointi-2025
        1601, // centria-csharp-basics-IT00AJ56-3008
    ];
    let mut client = TestMyCodeClient::new(
        "https://tmc.mooc.fi".parse().unwrap(),
        "vscode_plugin".to_string(),
        "3.4.0".to_string(),
    )
    .unwrap();

    let email = "daniel.x.martinez@helsinki.fi".to_string();
    let pass = rpassword::prompt_password("password").unwrap();
    client.authenticate(email, pass).unwrap();

    let root = Path::new("../../test-cache");
    let solutions_dir = root.join("solutions");
    let templates_dir = root.join("templates");
    let submissions_dir = root.join("submissions");
    std::fs::create_dir_all(&solutions_dir).unwrap();
    std::fs::create_dir_all(&templates_dir).unwrap();

    let mut failure = false;
    for course in courses.iter().copied() {
        println!("course {course}");
        let exercises = client.get_course_exercises(course).unwrap();
        for exercise in exercises {
            println!("course {course} exercise {}", exercise.id);
            // download template
            let template_path = templates_dir.join(exercise.id.to_string());
            let template_zip_path = template_path.with_extension("zip");
            let template_zip = match File::open(&template_zip_path) {
                Ok(file) => file,
                _ => {
                    let mut template_file = OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(&template_zip_path)
                        .unwrap();
                    client
                        .download_exercise(exercise.id, &mut template_file)
                        .unwrap();
                    template_file.seek(std::io::SeekFrom::Start(0)).unwrap();
                    template_file
                }
            };
            // extract template
            if !template_path.exists() {
                if let Err(err) = extract_to_new_dir(&template_zip, &template_path) {
                    let _ = std::fs::remove_dir(&template_path);
                    failure = true;
                    println!("error: while extracting template {err:#?}");
                    continue;
                }
            }
            // run tests on template as sanity check
            match tmc_langs::run_tests(&template_path) {
                Ok(test_results) => {
                    if matches!(test_results.status, RunStatus::Passed) {
                        println!(
                            "warning: course {course} exercise {} ({}) tests succeeded on the template",
                            exercise.name, exercise.id
                        );
                    } else {
                        println!(
                            "OK! course {course} exercise {} ({}) tests failed on the template",
                            exercise.name, exercise.id
                        )
                    }
                }
                Err(err) => {
                    failure = true;
                    println!("error: failed to run tests on example solution, {err:#?}")
                }
            }

            // download solution
            let solution_path = solutions_dir.join(exercise.id.to_string());
            let solution_zip_path = solution_path.with_extension("zip");
            let solution_zip = match File::open(&solution_zip_path) {
                Ok(file) => file,
                _ => {
                    let mut solution_zip = OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(&solution_zip_path)
                        .unwrap();
                    client
                        .download_model_solution_archive(exercise.id, &mut solution_zip)
                        .unwrap();
                    solution_zip.seek(std::io::SeekFrom::Start(0)).unwrap();
                    solution_zip
                }
            };
            // extract solution
            if !solution_path.exists() {
                if let Err(err) = extract_to_new_dir(&solution_zip, &solution_path) {
                    let _ = std::fs::remove_dir(&solution_path);
                    failure = true;
                    println!("error: while extracting solution {err:#?}");
                    continue;
                }
            }
            // run tests as sanity check
            match tmc_langs::run_tests(&solution_path) {
                Ok(test_results) => {
                    if !matches!(test_results.status, RunStatus::Passed) {
                        println!(
                            "warning: course {course} exercise {} ({}) tests failed on the example solution",
                            exercise.name, exercise.id
                        );
                    } else {
                        println!(
                            "OK! course {course} exercise {} ({}) tests passed on the example solution",
                            exercise.name, exercise.id
                        );
                    }
                }
                Err(err) => {
                    failure = true;
                    println!("error: failed to run tests on example solution, {err:#?}")
                }
            }

            // package solution as submission and extract over template
            let submission_dir = submissions_dir.join(exercise.id.to_string());
            let submission_path = submissions_dir.join(exercise.id.to_string());
            let submission_zip_path = submission_path.with_extension("zip");
            if !submission_zip_path.exists() {
                solution_into_submission(&template_path, &solution_zip, &submission_zip_path);
            };
            // extract submission
            if !submission_path.exists() {
                let submission_zip = File::open(&submission_zip_path).unwrap();
                if let Err(err) = extract_to_new_dir(&submission_zip, &submission_path) {
                    let _ = std::fs::remove_dir(&submission_path);
                    failure = true;
                    println!("error: while extracting solution {err:#?}");
                    continue;
                }
            }
            // run tests, should pass
            match tmc_langs::run_tests(&submission_dir) {
                Ok(test_results) => {
                    if !matches!(test_results.status, RunStatus::Passed) {
                        failure = true;
                        println!(
                            "error: course {course} exercise {} ({}) tests failed on solution extracted over template",
                            exercise.name, exercise.id
                        );
                    } else {
                        println!(
                            "OK! course {course} exercise {} ({}) tests passed on submission",
                            exercise.name, exercise.id
                        );
                    }
                }
                Err(err) => {
                    failure = true;
                    println!("error: failed to run tests on submission, {err:#?}")
                }
            }
        }
    }
    if failure {
        panic!("one or more issues found, check logs for more info");
    }
}

fn solution_into_submission(template: &Path, solution_zip: &File, target: &Path) {
    // extract solution
    let solution = TempDir::new().unwrap();
    tmc_langs::extract_project(
        solution_zip,
        solution.path(),
        Compression::Zip,
        false,
        false,
    )
    .unwrap();
    // package solution as a student submission
    let submission_zip = NamedTempFile::new().unwrap();
    tmc_langs::compress_project_to(
        solution.path(),
        submission_zip.path(),
        Compression::Zip,
        true,
        false,
    )
    .unwrap();

    tmc_langs::prepare_submission(
        tmc_langs::PrepareSubmission {
            archive: submission_zip.path(),
            compression: Compression::Zip,
            extract_naively: false,
        },
        target,
        true,
        tmc_langs::TmcParams::default(),
        template,
        None,
        Compression::Zip,
    )
    .unwrap();
}

fn extract_to_new_dir(zip: &File, path: &Path) -> Result<(), LangsError> {
    file_util::create_dir(path)?;
    tmc_langs::extract_project(zip, path, Compression::Zip, false, false)?;
    Ok(())
}
