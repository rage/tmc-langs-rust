//! Integration tests using the courses from TMC's test organization
//! Requires EMAIL and PASSWORD to be defined in tmc-client/.env

use dotenv::dotenv;
use std::env;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tmc_client::{ClientError, Exercise, SubmissionProcessingStatus, SubmissionStatus, TmcClient};
use tmc_langs_util::{Language, RunStatus};
use url::Url;

const TMC_ROOT: &str = "https://tmc.mooc.fi/";

fn init() {
    use log::*;
    use simple_logger::*;
    let _ = SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .with_module_level("j4rs", LevelFilter::Warn)
        .with_module_level("reqwest", LevelFilter::Warn)
        .with_module_level("hyper", LevelFilter::Warn)
        .with_module_level("serde_xml_rs", LevelFilter::Warn)
        .with_module_level("rustls", LevelFilter::Warn)
        .init();
    dotenv().ok();
}

fn authenticated_client() -> TmcClient {
    let email = env::var("EMAIL").unwrap();
    let password = env::var("PASSWORD").unwrap();
    let mut client = TmcClient::new_in_config(
        TMC_ROOT.to_string(),
        "vscode_plugin".to_string(),
        "test".to_string(),
    )
    .unwrap();
    client
        .authenticate("vscode_plugin", email, password)
        .unwrap();
    client
}

// downloads and submits all exercises for course, asserts that the tests are run but fail
fn dl_test_submit_course_templates(course_id: usize) {
    init();

    fn submitter(client: &TmcClient, exercise: Exercise) {
        let id = exercise.id;
        dl_test_submit_exercise(&client, exercise, |target| {
            client.download_or_update_exercises(vec![(id, target)])
        });
    }

    dl_test_submit_course_exercises(course_id, submitter);
}

// downloads and submits all exercise solutions for course, asserts that tests pass
fn dl_test_submit_course_solutions(course_id: usize) {
    init();

    fn submitter(client: &TmcClient, exercise: Exercise) {
        let solution_url = Url::parse(&exercise.return_url)
            .unwrap()
            .join("solution/download")
            .unwrap();
        dl_test_submit_exercise(&client, exercise, |target| {
            client.download_model_solution(solution_url, &target)
        });
    }

    dl_test_submit_course_exercises(course_id, submitter);
}

// fetches course exercises and runs submitter on each one
// tester_submitter should test and submit the exercise
fn dl_test_submit_course_exercises<F>(course_id: usize, tester_submitter: F)
where
    F: Fn(&TmcClient, Exercise),
{
    log::debug!("fetching course {}", course_id);
    let client = authenticated_client();
    let course_details = client.get_course_details(course_id).unwrap();
    log::debug!(
        "testing and submitting course templates for {:#?}",
        course_details
    );

    for exercise in course_details.exercises {
        if exercise.name.contains("osa12")
            || exercise.name.contains("osa13")
            || exercise.name.contains("osa14")
        {
            // java mooc requires javafx
            continue;
        }
        if [94743, 94765, 94800].contains(&exercise.id) {
            // bugged template
            continue;
        }
        if [95097, 95123].contains(&exercise.id) {
            // bugged solution
            continue;
        }

        tester_submitter(&client, exercise);
    }
}

// submits the exercise
// downloader should download the submission target to the path arg
fn dl_test_submit_exercise<F>(client: &TmcClient, exercise: Exercise, downloader: F)
where
    F: FnOnce(PathBuf) -> Result<(), ClientError>,
{
    log::debug!("submitting exercise {:#?}", exercise);
    let temp = tempfile::tempdir().unwrap();
    let submission_path = temp.path().join(exercise.id.to_string());
    log::debug!("downloading to {}", submission_path.display());
    downloader(submission_path.clone()).unwrap();

    log::debug!("testing locally {}", submission_path.display());
    let test_results = client.run_tests(&submission_path, &mut vec![]).unwrap();
    let expected = test_results.status;
    log::debug!("expecting {:?}", expected);

    let submission_url = Url::parse(&exercise.return_url).unwrap();
    log::debug!("submitting to {}", submission_url);
    let submission = client
        .submit(submission_url, &submission_path, Some(Language::Eng))
        .unwrap();
    log::debug!("got {:#?}", submission);

    log::debug!("waiting for submission to finish");
    let finished = loop {
        let status = client.check_submission(&submission.submission_url).unwrap();
        match status {
            SubmissionProcessingStatus::Finished(finished) => break *finished,
            SubmissionProcessingStatus::Processing(_) => thread::sleep(Duration::from_secs(2)),
        }
    };
    log::debug!("got {:#?}", finished);
    match expected {
        RunStatus::Passed => {
            assert_eq!(finished.status, SubmissionStatus::Ok);
            assert!(finished.all_tests_passed.unwrap());
        }
        RunStatus::TestsFailed => {
            assert_eq!(finished.status, SubmissionStatus::Fail);
            assert!(!finished.all_tests_passed.unwrap());
        }
        RunStatus::CompileFailed => {
            assert_eq!(finished.status, SubmissionStatus::Error);
            assert!(!finished.all_tests_passed.unwrap());
        }
        _ => panic!("something went wrong"),
    }
}

mod python {
    use super::*;

    const PYTHON_COURSE_ID: usize = 597;

    #[test]
    #[ignore]
    // passed 29.6.2020
    fn templates() {
        dl_test_submit_course_templates(PYTHON_COURSE_ID)
    }

    #[test]
    #[ignore]
    // passed 29.6.2020
    fn solutions() {
        dl_test_submit_course_solutions(PYTHON_COURSE_ID)
    }
}

mod java {
    use super::*;

    const JAVA_COURSE_ID: usize = 665;

    #[test]
    #[ignore]
    // passed 30.6.2020
    fn templates() {
        dl_test_submit_course_templates(JAVA_COURSE_ID)
    }

    #[test]
    #[ignore]
    // passed 30.6.2020
    fn solutions() {
        dl_test_submit_course_solutions(JAVA_COURSE_ID)
    }
}

mod r {
    use super::*;

    const R_COURSE_ID: usize = 0; // TODO

    #[test]
    #[ignore]
    fn templates() {
        dl_test_submit_course_templates(R_COURSE_ID)
    }

    #[test]
    #[ignore]
    fn solutions() {
        dl_test_submit_course_solutions(R_COURSE_ID)
    }
}

mod make {
    use super::*;

    const C_COURSE_ID: usize = 668;

    #[test]
    #[ignore]
    // failed due to invalid utf8 in test msg, should be fixed now
    fn templates() {
        dl_test_submit_course_templates(C_COURSE_ID)
    }

    #[test]
    #[ignore]
    // passed 1.7.2020
    fn solutions() {
        dl_test_submit_course_solutions(C_COURSE_ID)
    }
}

mod notests {
    use super::*;

    const NOTESTS_COURSE_ID: usize = 0; // TODO

    #[test]
    #[ignore]
    fn templates() {
        dl_test_submit_course_templates(NOTESTS_COURSE_ID)
    }

    #[test]
    #[ignore]
    fn solutions() {
        dl_test_submit_course_solutions(NOTESTS_COURSE_ID)
    }
}
