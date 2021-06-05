//! This library can be used to easily set up a mock tmc-server for testing.

pub use mockito;

use mockito::{mock, Matcher::*, Mock};
use serde_json::json;

/// Formats a mock config with the given projects dir and a mock setting with key "setting" and value "value".
/// This should be in TMC_LANGS_CONFIG_DIR/client/config.toml
pub fn mock_config(projects_dir: &str) -> String {
    format!(
        r#"
projects-dir = '{}'
setting = "value"
"#,
        projects_dir
    )
}

/// Formats a mock course config with 4 exercises.
/// This should be in projects-dir/client/mock-course/course_config.toml
pub fn mock_course_config() -> String {
    format!(
        r#"
course = '{}'

[exercises."{}"]
id = {}
checksum = '{checksum}'

[exercises."{}"]
id = {}
checksum = '{checksum}'

[exercises."{}"]
id = {}
checksum = '{checksum}'

[exercises."{}"]
id = {}
checksum = '{checksum}'
"#,
        COURSE_NAME,
        EXERCISE_NAMES[0],
        EXERCISE_IDS[0],
        EXERCISE_NAMES[1],
        EXERCISE_IDS[1],
        EXERCISE_NAMES[2],
        EXERCISE_IDS[2],
        EXERCISE_NAMES[3],
        EXERCISE_IDS[3],
        checksum = EXERCISE_CHECKSUM,
    )
}

pub const APPLICATION_NAME: &str = "mock-plugin";
pub const USER_ID: u32 = 1;
pub const ORGANIZATION_ID: u32 = 1;
pub const ORGANIZATION_NAME: &str = "mock-organization";
pub const ORGANIZATION_SLUG: &str = "morg";
pub const COURSE_ID: u32 = 1;
pub const COURSE_NAME: &str = "mock-course";
pub const EXERCISE_IDS: [u32; 4] = [1, 2, 3, 4];
pub const EXERCISE_NAMES: [&str; 4] = [
    "mock-exercise-1",
    "mock-exercise-2",
    "mock-exercise-3",
    "mock-exercise-4",
];
pub const EXERCISE_CHECKSUM: &str = "new checksum";
pub const SUBMISSION_ID: u32 = 1;
pub const EXERCISE_BYTES: &[u8] = include_bytes!("../python-exercise.zip");

macro_rules! mocker {
    ($m: tt, $r: tt, $j: tt) => {
        mock($m, Regex(format!("{}[^/]+$", $r.replace("{}", "[^/]+"))))
            .with_body(json!($j).to_string())
            .create()
    };
}

macro_rules! mocker_json {
    ($m: tt, $r: tt, $j: expr) => {
        mock($m, Regex(format!("{}[^/]+$", $r.replace("{}", "[^/]+"))))
            .with_body($j.to_string())
            .create()
    };
}

macro_rules! user {
    () => {json!({
        "id": USER_ID,
        "username": "student",
        "email": "student@helsinki.fi",
        "administrator": false
      })}
}

macro_rules! users {
    () => {json!([{
        "id": USER_ID,
        "username": "student",
        "email": "student@helsinki.fi",
        "administrator": false
      }])}
}

macro_rules! course {
    () => {json!({
      "name": COURSE_NAME,
      "hide_after": null,
      "hidden": false,
      "cache_version": 3,
      "spreadsheet_key": null,
      "hidden_if_registered_after": null,
      "refreshed_at": "2020-10-06T09:23:49.781+03:00",
      "locked_exercise_points_visible": true,
      "paste_visibility": null,
      "formal_name": null,
      "certificate_downloadable": false,
      "certificate_unlock_spec": null,
      "organization_id": ORGANIZATION_ID,
      "disabled_status": "enabled",
      "title": "Java Programming fall 2020",
      "description": "Course for the students of university of helsinki",
      "material_url": "",
      "course_template_id": 1,
      "hide_submission_results": false,
      "external_scoreboard_url": "",
      "organization_slug": ORGANIZATION_SLUG,
    })}
}

macro_rules! points {
    () => {json!([{
        "awarded_point": {
          "id": 123,
          "course_id": COURSE_ID,
          "user_id": USER_ID,
          "submission_id": SUBMISSION_ID,
          "name": "01",
          "created_at": "2021-06-18T02:33:36.442+03:00"
        },
        "exercise_id": EXERCISE_IDS[0]
    }])}
}

macro_rules! submissions {
    () => {json!([{
        "id": SUBMISSION_ID,
        "user_id": USER_ID,
        "pretest_error": null,
        "created_at": "2021-05-20T07:48:41.671+03:00",
        "exercise_name": EXERCISE_NAMES[0],
        "course_id": COURSE_ID,
        "processed": true,
        "all_tests_passed": false,
        "points": null,
        "processing_tried_at": "2021-05-20T07:48:41.748+03:00",
        "processing_began_at": "2021-05-20T07:48:41.890+03:00",
        "processing_completed_at": "2021-05-20T07:49:06.368+03:00",
        "times_sent_to_sandbox": 1,
        "processing_attempts_started_at": "2021-05-20T07:48:41.671+03:00",
        "params_json": "{\"error_msg_locale\":null}",
        "requires_review": false,
        "requests_review": false,
        "reviewed": false,
        "message_for_reviewer": "",
        "newer_submission_reviewed": false,
        "review_dismissed": false,
        "paste_available": true,
        "message_for_paste": "test",
        "paste_key": null
    }])}
}

macro_rules! exercises {
    () => {json!([{
        "id": EXERCISE_IDS[0],
        "available_points": [
        {
            "id": 123,
            "exercise_id": EXERCISE_IDS[0],
            "name": "01",
            "requires_review": false
        }
        ],
        "awarded_points": [],
        "name": EXERCISE_NAMES[0],
        "publish_time": null,
        "solution_visible_after": null,
        "deadline": "2020-11-26T23:59:59.999+02:00",
        "soft_deadline": null,
        "disabled": false,
        "unlocked": true
    }])}
}

pub fn get_credentials() -> Mock {
    mocker!("GET", "/api/v8/application/{}/credentials", {
        "application_id": "1234",
        "secret": "abcd"
    })
}

pub fn get_submission() -> Mock {
    mocker!("GET", "/api/v8/core/submission/{}", {
        "api_version": 0,
        "user_id": USER_ID,
        "login": "",
        "course": COURSE_NAME,
        "exercise_name": EXERCISE_NAMES[0],
        "status": "processing",
        "points": [],
        "submission_url": "",
        "submitted_at": "",
        "reviewed": false,
        "requests_review": false,
        "missing_review_points": []
    })
}

pub mod user {
    use super::*;

    pub fn get() -> Mock {
        mocker_json!("GET", "/api/v8/users/{}", users!())
    }

    pub fn get_current() -> Mock {
        mocker_json!("GET", "/api/v8/users/current", user!())
    }

    pub fn get_basic_info_by_usernames() -> Mock {
        mocker_json!("GET", "/api/v8/users/basic_info_by_usernames", users!())
    }

    pub fn get_basic_info_by_emails() -> Mock {
        mocker_json!("GET", "/api/v8/users/basic_info_by_emails", users!())
    }
}

pub mod course {
    use super::*;

    pub fn get_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}", course!())
    }

    pub fn get() -> Mock {
        mocker_json!("GET", "/api/v8/org/{}/courses/{}", course!())
    }
}

pub mod point {
    use super::*;

    pub fn get_course_points_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/points", points!())
    }

    pub fn get_exercise_points_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/exercises/{}/points", points!())
    }

    pub fn get_exercise_points_for_user_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/users/{}/points", points!())
    }

    pub fn get_exercise_points_for_current_user_by_id() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/courses/{}/exercises/{}/users/current/points",
            points!()
        )
    }

    pub fn get_course_points_for_user_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/users/{}/points", points!())
    }

    pub fn get_course_points_for_current_user_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/users/current/points", points!())
    }

    pub fn get_course_points() -> Mock {
        mocker_json!("GET", "/api/v8/org/{}/courses/{}/points", points!())
    }

    pub fn get_exercise_points() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/exercises/{}/points",
            points!()
        )
    }

    pub fn get_course_points_for_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/users/{}/points",
            points!()
        )
    }

    pub fn get_course_points_for_current_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/users/current/points",
            points!()
        )
    }
}

pub mod submission {
    use super::*;

    pub fn get_course_submissions_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/{}/submissions", submissions!())
    }

    pub fn get_course_submissions_for_last_hour() -> Mock {
        mocker!(
            "GET",
            "/api/v8/courses/{}/submissions/last_hour",
            [SUBMISSION_ID]
        )
    }

    pub fn get_course_submissions_for_user_by_id() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/courses/{}/users/{}/submissions",
            submissions!()
        )
    }

    pub fn get_course_submissions_for_current_user_by_id() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/courses/{}/users/current/submissions",
            submissions!()
        )
    }

    pub fn get_exercise_submissions_for_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/exercises/{}/users/{}/submissions",
            submissions!()
        )
    }

    pub fn get_exercise_submissions_for_current_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/exercises/{}/users/current/submissions",
            submissions!()
        )
    }

    pub fn get_course_submissions() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/submissions",
            submissions!()
        )
    }

    pub fn get_course_submissions_for_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/users/{}/submissions",
            submissions!()
        )
    }

    pub fn get_course_submissions_for_current_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/org/{}/courses/{}/users/current/submissions",
            submissions!()
        )
    }
}

pub mod exercise {
    use super::*;

    pub fn get_course_exercises_by_id() -> Mock {
        mocker_json!("GET", "/api/v8/courses/.*/exercises", exercises!())
    }

    pub fn get_exercise_submissions_for_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/exercises/{}/users/{}/submissions",
            submissions!()
        )
    }

    pub fn get_exercise_submissions_for_current_user() -> Mock {
        mocker_json!(
            "GET",
            "/api/v8/exercises/{}/users/current/submissions",
            submissions!()
        )
    }

    pub fn get_course_exercises() -> Mock {
        mocker_json!("GET", "/api/v8/org/{}/courses/{}/exercises", exercises!())
    }

    pub fn download_course_exercise() -> Mock {
        mock(
            "GET",
            Regex("/api/v8/org/{}/courses/{}/exercises/{}/download".replace("{}", "[^/]")),
        )
        .with_body(EXERCISE_BYTES)
        .create()
    }
}

pub mod organization {
    use super::*;

    pub fn get_organizations() -> Mock {
        mocker!("GET", "/api/v8/org.json", [{
          "id": ORGANIZATION_ID,
          "name": ORGANIZATION_NAME,
          "information": "info",
          "slug": ORGANIZATION_SLUG,
          "verified_at": null,
          "verified": true,
          "disabled": false,
          "disabled_reason": null,
          "created_at": "2015-08-03T17:06:45.307+03:00",
          "updated_at": "2017-12-15T18:45:14.546+02:00",
          "hidden": false,
          "creator_id": null,
          "logo_file_name": "logo.png",
          "logo_content_type": "image/png",
          "logo_file_size": 10,
          "logo_updated_at": "2017-12-15T18:45:10.017+02:00",
          "phone": "",
          "contact_information": "",
          "email": "",
          "website": "",
          "pinned": true,
          "whitelisted_ips": null,
          "logo_path": "logo.png"
        }])
    }

    pub fn get_organization() -> Mock {
        mocker!("GET", "/api/v8/org/{}.json", {
          "id": ORGANIZATION_ID,
          "name": ORGANIZATION_NAME,
          "information": "info",
          "slug": ORGANIZATION_SLUG,
          "verified_at": null,
          "verified": true,
          "disabled": false,
          "disabled_reason": null,
          "created_at": "2015-08-03T17:06:45.307+03:00",
          "updated_at": "2017-12-15T18:45:14.546+02:00",
          "hidden": false,
          "creator_id": null,
          "logo_file_name": "logo.png",
          "logo_content_type": "image/png",
          "logo_file_size": 10,
          "logo_updated_at": "2017-12-15T18:45:10.017+02:00",
          "phone": "",
          "contact_information": "",
          "email": "",
          "website": "",
          "pinned": true,
          "whitelisted_ips": null,
          "logo_path": "logo.png"
        })
    }
}

pub mod core {
    use super::*;

    pub fn get_course() -> Mock {
        mocker!("GET", "/api/v8/core/courses/{}", {
            "course": {
              "id": COURSE_ID,
              "name": COURSE_NAME,
              "title": "Java Programming fall 2020",
              "description": "Course for the students of university of helsinki",
              "details_url": "https://localhost/",
              "unlock_url": "https://localhost/",
              "reviews_url": "https://localhost/",
              "comet_url": "",
              "spyware_urls": [
                "http://localhost/"
              ],
              "unlockables": [],
              "exercises": [
                {
                    "id": EXERCISE_IDS[0],
                    "name": EXERCISE_NAMES[0],
                    "locked": false,
                    "deadline_description": "2016-02-29 23:59:00 +0200",
                    "deadline": "2016-02-29T23:59:00.000+02:00",
                    "checksum": "new checksum",
                    "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/submissions",
                    "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/4272/download",
                    "returnable": true,
                    "requires_review": false,
                    "attempted": false,
                    "completed": false,
                    "reviewed": false,
                    "all_review_points_given": true,
                    "memory_limit": 1024,
                    "runtime_params": [
                      "-Xss64M"
                    ],
                    "valgrind_strategy": "fail",
                    "code_review_requests_enabled": false,
                    "run_tests_locally_action_enabled": true,
                    "exercise_submissions_url": "https://localhost",
                    "latest_submission_url": "https://localhost",
                    "latest_submission_id": SUBMISSION_ID,
                    "solution_zip_url": "http://localhost"
                },
                {
                    "id": EXERCISE_IDS[1],
                    "name": EXERCISE_NAMES[1],
                    "locked": false,
                    "deadline_description": "2016-02-29 23:59:00 +0200",
                    "deadline": "2016-02-29T23:59:00.000+02:00",
                    "checksum": "new checksum",
                    "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/submissions",
                    "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/4272/download",
                    "returnable": true,
                    "requires_review": false,
                    "attempted": false,
                    "completed": false,
                    "reviewed": false,
                    "all_review_points_given": true,
                    "memory_limit": 1024,
                    "runtime_params": [
                      "-Xss64M"
                    ],
                    "valgrind_strategy": "fail",
                    "code_review_requests_enabled": false,
                    "run_tests_locally_action_enabled": true,
                    "exercise_submissions_url": "https://localhost",
                    "latest_submission_url": "https://localhost",
                    "latest_submission_id": SUBMISSION_ID,
                    "solution_zip_url": "http://localhost"
                },
                {
                    "id": EXERCISE_IDS[2],
                    "name": EXERCISE_NAMES[2],
                    "locked": false,
                    "deadline_description": "2016-02-29 23:59:00 +0200",
                    "deadline": "2016-02-29T23:59:00.000+02:00",
                    "checksum": "new checksum",
                    "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/submissions",
                    "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/4272/download",
                    "returnable": true,
                    "requires_review": false,
                    "attempted": false,
                    "completed": false,
                    "reviewed": false,
                    "all_review_points_given": true,
                    "memory_limit": 1024,
                    "runtime_params": [
                      "-Xss64M"
                    ],
                    "valgrind_strategy": "fail",
                    "code_review_requests_enabled": false,
                    "run_tests_locally_action_enabled": true,
                    "exercise_submissions_url": "https://localhost",
                    "latest_submission_url": "https://localhost",
                    "latest_submission_id": SUBMISSION_ID,
                    "solution_zip_url": "http://localhost"
                },
                {
                    "id": EXERCISE_IDS[3],
                    "name": EXERCISE_NAMES[3],
                    "locked": false,
                    "deadline_description": "2016-02-29 23:59:00 +0200",
                    "deadline": "2016-02-29T23:59:00.000+02:00",
                    "checksum": "new checksum",
                    "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/submissions",
                    "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/4272/download",
                    "returnable": true,
                    "requires_review": false,
                    "attempted": false,
                    "completed": false,
                    "reviewed": false,
                    "all_review_points_given": true,
                    "memory_limit": 1024,
                    "runtime_params": [
                      "-Xss64M"
                    ],
                    "valgrind_strategy": "fail",
                    "code_review_requests_enabled": false,
                    "run_tests_locally_action_enabled": true,
                    "exercise_submissions_url": "https://localhost",
                    "latest_submission_url": "https://localhost",
                    "latest_submission_id": SUBMISSION_ID,
                    "solution_zip_url": "http://localhost"
                },
              ]
        }})
    }

    pub fn get_course_reviews() -> Mock {
        mocker!("GET", "/api/v8/core/courses/{}/reviews", [
        {
          "submission_id": SUBMISSION_ID,
          "exercise_name": "part01-Part01_01.Sandbox",
          "id": 1,
          "marked_as_read": false,
          "reviewer_name": "1234",
          "review_body": "nice review",
          "points": [],
          "points_not_awarded": [],
          "url": "https://localhost/",
          "update_url": "https://localhost/",
          "created_at": "2021-05-18T02:56:26.667+03:00",
          "updated_at": "2021-05-18T23:41:49.605+03:00"
        }])
    }

    pub fn update_course_review() -> Mock {
        mocker!("PUT", "/api/v8/core/courses/{}/reviews/{}", {})
    }

    pub fn unlock_course() -> Mock {
        mocker!("POST", "/api/v8/core/courses/{}/unlock", {})
    }

    pub fn download_exercise() -> Mock {
        mock(
            "GET",
            Regex("/api/v8/core/exercises/[^/]+/download".to_string()),
        )
        .with_body(EXERCISE_BYTES)
        .create()
    }

    pub fn get_exercise() -> Mock {
        mocker!("GET", "/api/v8/core/exercises/[0-9]+", {
            "course_name": COURSE_NAME,
            "course_id": COURSE_ID,
            "code_review_requests_enabled": true,
            "run_tests_locally_action_enabled": true,
            "exercise_name": EXERCISE_NAMES[0],
            "exercise_id": EXERCISE_IDS[0],
            "unlocked_at": null,
            "deadline": "2020-11-26T23:59:59.999+02:00",
            "submissions": []
        })
    }

    pub fn get_exercise_details() -> Mock {
        mocker!("GET", "/api/v8/core/exercises/details", {
            "exercises": [
                {
                    "id": EXERCISE_IDS[0],
                    "course_name": COURSE_NAME,
                    "exercise_name": EXERCISE_NAMES[0],
                    "checksum": EXERCISE_CHECKSUM,
                },
                {
                    "id": EXERCISE_IDS[1],
                    "course_name": COURSE_NAME,
                    "exercise_name": EXERCISE_NAMES[1],
                    "checksum": EXERCISE_CHECKSUM,
                },
                {
                    "id": EXERCISE_IDS[2],
                    "course_name": COURSE_NAME,
                    "exercise_name": EXERCISE_NAMES[2],
                    "checksum": EXERCISE_CHECKSUM,
                },
                {
                    "id": EXERCISE_IDS[3],
                    "course_name": COURSE_NAME,
                    "exercise_name": EXERCISE_NAMES[3],
                    "checksum": EXERCISE_CHECKSUM,
                },
            ]
        })
    }

    pub fn download_exercise_solution() -> Mock {
        mock(
            "GET",
            Regex("/api/v8/core/exercises/[^/]+/solution/download".to_string()),
        )
        .with_body(EXERCISE_BYTES)
        .create()
    }

    pub fn submit_exercise() -> Mock {
        mocker!("POST", "/api/v8/core/exercises/{}/submissions", {
            "show_submission_url": "url",
            "paste_url": "url",
            "submission_url": "url"
        })
    }

    pub fn get_organization_courses() -> Mock {
        mocker!("GET", "/api/v8/core/org/{}/courses", [{
          "id": COURSE_ID,
          "name": COURSE_NAME,
          "title": "Data Analysis with Python 2020",
          "description": "TMC exercises for the course Data Analysis with Python 2020",
          "details_url": "https://localhost",
          "unlock_url": "https://localhost",
          "reviews_url": "https://localhost",
          "comet_url": "",
          "spyware_urls": ["http://localhost"]
        }])
    }

    pub fn download_submission() -> Mock {
        mock(
            "GET",
            Regex("/api/v8/core/submissions/[^/]+/download".to_string()),
        )
        .with_body(EXERCISE_BYTES)
        .create()
    }

    pub fn post_submission_feedback() -> Mock {
        mocker!("POST", "/api/v8/core/submissions/{}/feedback", {
            "api_version": 1,
            "status": "processing"
        })
    }

    pub fn post_submission_review() -> Mock {
        mocker!("POST", "/api/v8/core/submissions/{}/reviews", {})
    }
}

pub fn mock_all() -> Vec<Mock> {
    vec![
        get_credentials(),
        get_submission(),
        user::get(),
        user::get_current(),
        user::get_basic_info_by_usernames(),
        user::get_basic_info_by_emails(),
        course::get_by_id(),
        course::get(),
        point::get_course_points_by_id(),
        point::get_exercise_points_by_id(),
        point::get_exercise_points_for_user_by_id(),
        point::get_exercise_points_for_current_user_by_id(),
        point::get_course_points_for_user_by_id(),
        point::get_course_points_for_current_user_by_id(),
        point::get_course_points(),
        point::get_exercise_points(),
        point::get_course_points_for_user(),
        point::get_course_points_for_current_user(),
        submission::get_course_submissions_by_id(),
        submission::get_course_submissions_for_last_hour(),
        submission::get_course_submissions_for_user_by_id(),
        submission::get_course_submissions_for_current_user_by_id(),
        submission::get_exercise_submissions_for_user(),
        submission::get_exercise_submissions_for_current_user(),
        submission::get_course_submissions(),
        submission::get_course_submissions_for_user(),
        submission::get_course_submissions_for_current_user(),
        exercise::get_course_exercises_by_id(),
        exercise::get_exercise_submissions_for_user(),
        exercise::get_exercise_submissions_for_current_user(),
        exercise::get_course_exercises(),
        exercise::download_course_exercise(),
        organization::get_organizations(),
        organization::get_organization(),
        core::get_course(),
        core::get_course_reviews(),
        core::update_course_review(),
        core::unlock_course(),
        core::download_exercise(),
        core::get_exercise(),
        core::get_exercise_details(),
        core::download_exercise_solution(),
        core::submit_exercise(),
        core::get_organization_courses(),
        core::download_submission(),
        core::post_submission_feedback(),
        core::post_submission_review(),
    ]
}
