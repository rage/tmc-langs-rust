//! Integration tests using the courses from TMC's test organization
//! Requires EMAIL and PASSWORD to be defined in tmc-langs-core/.env

use dotenv::dotenv;
use std::env;
use std::path::Path;
use std::thread;
use std::time::Duration;
use tmc_langs_core::{
    CoreError, Exercise, SubmissionFinished, SubmissionProcessingStatus, TmcCore,
};
use tmc_langs_util::Language;
use url::Url;

const TMC_ROOT: &str = "https://tmc.mooc.fi/";

fn init() {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug,j4rs=warn,hyper=warn,reqwest=warn");
    }
    let _ = env_logger::builder().is_test(true).try_init();
    dotenv().ok();
}

fn authenticated_core() -> TmcCore {
    let email = env::var("EMAIL").unwrap();
    let password = env::var("PASSWORD").unwrap();
    let mut core = TmcCore::new_in_config(TMC_ROOT.to_string()).unwrap();
    core.authenticate("vscode_plugin", email, password).unwrap();
    core
}

// fetches course exercises and runs submitter on each one
// submitter should submit the exercise
// assert should assert something about each submission result
fn submit_downloaded_course_exercises<F, A>(course_id: usize, submitter: F, assert: A)
where
    F: Fn(&TmcCore, Exercise) -> SubmissionFinished,
    A: Fn(SubmissionFinished) -> (),
{
    log::debug!("fetching course {}", course_id);
    let core = authenticated_core();
    let course_details = core.get_course_details(course_id).unwrap();
    log::debug!("submitting course templates for {:#?}", course_details);

    for exercise in course_details.exercises {
        if [93659, 92964, 92960, 82587].contains(&exercise.id) {
            log::info!("skipping {}: solution does not pass tests", exercise.id);
            continue;
        }
        if exercise.name.contains("osa1") {
            // temp
            continue;
        }
        let finished = submitter(&core, exercise);
        assert(finished)
    }
}

// submits the exercise
// downloader should download the submission target to the path arg
fn submit_downloaded_exercise<F: FnOnce(&Path) -> Result<(), CoreError>>(
    core: &TmcCore,
    exercise: Exercise,
    downloader: F,
) -> SubmissionFinished {
    log::debug!("submitting exercise {:#?}", exercise);
    let temp = tempfile::tempdir().unwrap();
    let submission_path = temp.path().join(exercise.id.to_string());
    log::debug!("downloading to {}", submission_path.display());
    downloader(&submission_path).unwrap();

    let submission_url = Url::parse(&exercise.return_url).unwrap();
    log::debug!("submitting to {}", submission_url);
    let submission = core
        .submit(submission_url, &submission_path, Language::Eng)
        .unwrap();
    log::debug!("got {:#?}", submission);

    log::debug!("waiting for submission to finish");
    let finished = loop {
        let status = core.check_submission(&submission.submission_url).unwrap();
        match status {
            SubmissionProcessingStatus::Finished(finished) => break *finished,
            SubmissionProcessingStatus::Processing(_) => thread::sleep(Duration::from_secs(2)),
        }
    };
    log::debug!("got {:#?}", finished);
    finished
}

mod python {
    use super::*;

    const PYTHON_COURSE_ID: usize = 597;

    #[test]
    #[ignore]
    // passed 29.6.2020
    fn submit_python_course_templates() {
        init();

        fn submitter(core: &TmcCore, exercise: Exercise) -> SubmissionFinished {
            let id = exercise.id;
            submit_downloaded_exercise(&core, exercise, |target| {
                core.download_or_update_exercises(vec![(id, target)])
            })
        }

        submit_downloaded_course_exercises(PYTHON_COURSE_ID, submitter, |finished| {
            assert!(!finished.all_tests_passed.unwrap())
        });
    }

    #[test]
    #[ignore]
    fn submit_python_course_solutions() {
        init();

        fn submitter(core: &TmcCore, exercise: Exercise) -> SubmissionFinished {
            let solution_url = Url::parse(&exercise.return_url)
                .unwrap()
                .join("solution/download")
                .unwrap();
            submit_downloaded_exercise(&core, exercise, |target| {
                core.download_model_solution(solution_url, target)
            })
        }

        submit_downloaded_course_exercises(PYTHON_COURSE_ID, submitter, |finished| {
            assert!(finished.all_tests_passed.unwrap())
        });
    }
}
