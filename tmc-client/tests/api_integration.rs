use dotenv::dotenv;
use std::{env, io::Read};
use tmc_client::{api_v8, FeedbackAnswer, TmcClient};

const ORGANIZATION_SLUG: &str = "hy";
const COURSE_NAME: &str = "java-1-f2020";
const EXERCISE_NAME: &str = "part01-Part01_01.Sandbox";
const COURSE_ID: u32 = 714;
const EXERCISE_ID: u32 = 104347;
const SUBMISSION_ID: u32 = 10406006;
const SUBMISSION_ZIP: &[u8] = include_bytes!("data/part01-Part01_01.Sandbox.zip");
const REVIEW_ID: u32 = 1138;

fn init_client() -> TmcClient {
    use log::*;
    use simple_logger::*;
    let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();

    dotenv().ok();
    let email = env::var("TMC_EMAIL").unwrap();
    let password = env::var("TMC_PASSWORD").unwrap();

    let mut client = TmcClient::new(
        "https://tmc.mooc.fi".parse().unwrap(),
        "vscode_plugin".to_string(),
        "1.0.0".to_string(),
    );
    client
        .authenticate("vscode_plugin", email, password)
        .unwrap();
    client
}

#[test]
#[ignore]
fn gets_user() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let _res = api_v8::user::get(client, user_id).unwrap();
}

#[test]
#[ignore]
fn gets_current_user() {
    let client = &init_client();
    let _res = api_v8::user::get_current(client).unwrap();
}

#[test]
#[ignore]
fn gets_basic_info_by_usernames() {
    let client = &init_client();
    let username = env::var("TMC_NAME").unwrap();
    let res =
        api_v8::user::get_basic_info_by_usernames(client, &[username.clone(), username]).unwrap();
    assert!(!res.is_empty())
}

#[test]
#[ignore]
fn gets_basic_info_by_emails() {
    let client = &init_client();
    let email = env::var("TMC_EMAIL").unwrap();
    let res = api_v8::user::get_basic_info_by_emails(client, &[email.clone(), email]).unwrap();
    assert!(!res.is_empty())
}

#[test]
#[ignore]
fn gets_course_by_id() {
    let client = &init_client();
    let _res = api_v8::course::get_by_id(client, COURSE_ID).unwrap();
}

#[test]
#[ignore]
fn gets_course() {
    let client = &init_client();
    let _res = api_v8::course::get(client, ORGANIZATION_SLUG, COURSE_NAME).unwrap();
}

#[test]
#[ignore]
fn gets_course_points_by_id() {
    let client = &init_client();
    let res = api_v8::point::get_course_points_by_id(client, COURSE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_points_by_id() {
    let client = &init_client();
    let res = api_v8::point::get_exercise_points_by_id(client, COURSE_ID, EXERCISE_NAME).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_points_for_user_by_id() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res = api_v8::point::get_exercise_points_for_user_by_id(
        client,
        COURSE_ID,
        EXERCISE_NAME,
        user_id,
    )
    .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_points_for_current_user_by_id() {
    let client = &init_client();
    let res =
        api_v8::point::get_exercise_points_for_current_user_by_id(client, COURSE_ID, EXERCISE_NAME)
            .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_points_for_user_by_id() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res = api_v8::point::get_course_points_for_user_by_id(client, COURSE_ID, user_id).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_points_for_current_user_by_id() {
    let client = &init_client();
    let res = api_v8::point::get_course_points_for_current_user_by_id(client, COURSE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_points() {
    let client = &init_client();
    let res = api_v8::point::get_course_points(client, ORGANIZATION_SLUG, COURSE_NAME).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_points() {
    let client = &init_client();
    let res =
        api_v8::point::get_exercise_points(client, ORGANIZATION_SLUG, COURSE_NAME, EXERCISE_NAME)
            .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_points_for_user() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res =
        api_v8::point::get_course_points_for_user(client, ORGANIZATION_SLUG, COURSE_NAME, user_id)
            .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_points_for_current_user() {
    let client = &init_client();
    let res =
        api_v8::point::get_course_points_for_current_user(client, ORGANIZATION_SLUG, COURSE_NAME)
            .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_by_id() {
    let client = &init_client();
    let res = api_v8::submission::get_course_submissions_by_id(client, COURSE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_for_last_hour() {
    let client = &init_client();
    let res = api_v8::submission::get_course_submissions_for_last_hour(client, COURSE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_for_user_by_id() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res = api_v8::submission::get_course_submissions_for_user_by_id(client, COURSE_ID, user_id)
        .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_for_current_user_by_id() {
    let client = &init_client();
    let res = api_v8::submission::get_course_submissions_for_current_user_by_id(client, COURSE_ID)
        .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_submissions_for_user_by_id() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res = api_v8::submission::get_exercise_submissions_for_user(client, EXERCISE_ID, user_id)
        .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_submissions_for_current_user_by_id() {
    let client = &init_client();
    let res =
        api_v8::submission::get_exercise_submissions_for_current_user(client, EXERCISE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions() {
    let client = &init_client();
    let res =
        api_v8::submission::get_course_submissions(client, ORGANIZATION_SLUG, COURSE_NAME).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_for_user() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res = api_v8::submission::get_course_submissions_for_user(
        client,
        ORGANIZATION_SLUG,
        COURSE_NAME,
        user_id,
    )
    .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_submissions_for_current_user() {
    let client = &init_client();
    let res = api_v8::submission::get_course_submissions_for_current_user(
        client,
        ORGANIZATION_SLUG,
        COURSE_NAME,
    )
    .unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_exercises_by_id() {
    let client = &init_client();
    let res = api_v8::exercise::get_course_exercises_by_id(client, COURSE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_submissions_for_user() {
    let client = &init_client();
    let user_id: u32 = env::var("TMC_USER_ID").unwrap().parse().unwrap();
    let res =
        api_v8::exercise::get_exercise_submissions_for_user(client, EXERCISE_ID, user_id).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_exercise_submissions_for_current_user() {
    let client = &init_client();
    let res =
        api_v8::exercise::get_exercise_submissions_for_current_user(client, EXERCISE_ID).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_course_exercises() {
    let client = &init_client();
    let res =
        api_v8::exercise::get_course_exercises(client, ORGANIZATION_SLUG, COURSE_NAME).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn downloads_course_exercise() {
    let client = &init_client();
    let mut temp = tempfile::tempfile().unwrap();
    api_v8::exercise::download_course_exercise(
        client,
        ORGANIZATION_SLUG,
        COURSE_NAME,
        EXERCISE_NAME,
        &mut temp,
    )
    .unwrap();
    let mut buf = vec![];
    temp.read_to_end(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
#[ignore]
fn gets_organizations() {
    let client = &init_client();
    let res = api_v8::organization::get_organizations(client).unwrap();
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn gets_organization() {
    let client = &init_client();
    let _res = api_v8::organization::get_organization(client, ORGANIZATION_SLUG).unwrap();
}

#[test]
#[ignore]
fn gets_core_course() {
    let client = &init_client();
    let _res = api_v8::core::get_course(client, COURSE_ID).unwrap();
}

#[test]
#[ignore]
fn gets_course_reviews() {
    let client = &init_client();
    let res = api_v8::core::get_course_reviews(client, COURSE_ID).unwrap();
    assert!(!res.is_empty())
}

#[test]
#[ignore]
fn puts_course_review() {
    let client = &init_client();
    api_v8::core::update_course_review(
        client,
        COURSE_ID,
        REVIEW_ID,
        Some("update".to_string()),
        Some(true),
    )
    .unwrap();
}

#[test]
#[ignore]
fn downloads_core_exercise() {
    let client = &init_client();
    let mut temp = tempfile::tempfile().unwrap();
    api_v8::core::download_exercise(client, EXERCISE_ID, &mut temp).unwrap();
    let mut buf = vec![];
    temp.read_to_end(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
#[ignore]
fn gets_core_exercise() {
    let client = &init_client();
    let _res = api_v8::core::get_exercise(client, EXERCISE_ID).unwrap();
}

#[test]
#[ignore]
fn gets_exercise_details() {
    let client = &init_client();
    let res = api_v8::core::get_exercise_details(client, &[EXERCISE_ID]).unwrap();
    assert!(!res.is_empty())
}

#[test]
#[ignore]
fn downloads_exercise_solution() {
    let client = &init_client();
    let mut temp = tempfile::tempfile().unwrap();
    api_v8::core::download_exercise_solution(client, EXERCISE_ID, &mut temp).unwrap();
    let mut buf = vec![];
    temp.read_to_end(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
#[ignore]
fn submits_exercise() {
    let client = &init_client();
    let _res = api_v8::core::submit_exercise(client, EXERCISE_ID, SUBMISSION_ZIP, None, None, None)
        .unwrap();
}

#[test]
#[ignore]
fn gets_organization_courses() {
    let client = &init_client();
    let res = api_v8::core::get_organization_courses(client, ORGANIZATION_SLUG).unwrap();
    assert!(!res.is_empty())
}

#[test]
#[ignore]
fn downloads_submission() {
    let client = &init_client();
    let mut temp = tempfile::tempfile().unwrap();
    api_v8::core::download_submission(client, SUBMISSION_ID, &mut temp).unwrap();
    let mut buf = vec![];
    temp.read_to_end(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
#[ignore]
fn posts_submission_feedback() {
    let client = &init_client();
    let feedback = vec![
        FeedbackAnswer {
            question_id: 389,
            answer: "3".to_string(),
        },
        FeedbackAnswer {
            question_id: 390,
            answer: "3".to_string(),
        },
        FeedbackAnswer {
            question_id: 391,
            answer: "3".to_string(),
        },
        FeedbackAnswer {
            question_id: 392,
            answer: "3".to_string(),
        },
    ];
    let _res = api_v8::core::post_submission_feedback(client, 7402793, feedback).unwrap();
}

#[test]
#[ignore]
fn posts_submission_review() {
    let client = &init_client();
    let _res = api_v8::core::post_submission_review(
        client,
        SUBMISSION_ID,
        "review body".to_string(),
        // &["01-01".to_string()],
    )
    .unwrap();
}
