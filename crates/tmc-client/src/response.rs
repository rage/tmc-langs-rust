//! Contains types which model the JSON responses from tmc-server

use chrono::{DateTime, FixedOffset};
use once_cell::sync::Lazy;
use regex::Regex;
use schemars::JsonSchema;
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{fmt, str::FromStr};
use thiserror::Error;
use tmc_langs_plugins::StyleValidationResult;

/// Represents an error response from tmc-server
#[derive(Debug, Error, Deserialize)]
#[error("Response contained errors: {error:?}, {errors:#?}, obsolete client: {obsolete_client}")]
#[serde(deny_unknown_fields)] // prevents responses with an errors field from being parsed as an error
pub struct ErrorResponse {
    pub status: Option<String>,
    pub error: Option<String>,
    pub errors: Option<Vec<String>>,
    #[serde(default)]
    pub obsolete_client: bool,
}

/// OAuth2 credentials.
/// get /api/v8/application/{client_name}/credentials
#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub application_id: String,
    pub secret: String,
}

/// get /api/v8/users/{user_id}
/// get /api/v8/users/current
/// post /api/v8/users/basic_info_by_usernames
/// post /api/v8/users/basic_info_by_emails
#[derive(Debug, Deserialize)]
pub struct User {
    pub id: u32,
    pub username: String,
    pub email: String,
    pub administrator: bool,
}

/// get /api/v8/org.json
/// get /api/v8/org/{organization_slug}.json
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Organization {
    pub name: String,
    pub information: String,
    pub slug: String,
    pub logo_path: String,
    pub pinned: bool,
}

/// get /api/v8/core/org/{organization_slug}/courses
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Course {
    pub id: u32,
    pub name: String,
    pub title: String,
    pub description: Option<String>,
    /// /api/v8/core/courses/{course_id}
    pub details_url: String,
    /// /api/v8/core/courses/{course_id}/unlock
    pub unlock_url: String,
    /// /api/v8/core/courses/{course_id}/reviews
    pub reviews_url: String,
    /// Typically empty.
    pub comet_url: String,
    pub spyware_urls: Vec<String>,
}

/// get /api/v8/courses/{course_id}
/// get /api/v8/org/{organization_slug}/courses/{course_name}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct CourseData {
    pub name: String,
    pub hide_after: Option<String>,
    pub hidden: bool,
    pub cache_version: Option<u32>,
    pub spreadsheet_key: Option<String>,
    pub hidden_if_registered_after: Option<String>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string | null"))]
    pub refreshed_at: Option<DateTime<FixedOffset>>,
    pub locked_exercise_points_visible: bool,
    pub description: Option<String>,
    pub paste_visibility: Option<u32>,
    pub formal_name: Option<String>,
    pub certificate_downloadable: Option<bool>,
    pub certificate_unlock_spec: Option<String>,
    pub organization_id: Option<u32>,
    pub disabled_status: Option<String>,
    pub title: Option<String>,
    /// Typically empty.
    pub material_url: Option<String>,
    pub course_template_id: Option<u32>,
    pub hide_submission_results: bool,
    /// Typically empty.
    pub external_scoreboard_url: Option<String>,
    pub organization_slug: Option<String>,
}

/// Represents a course details response from tmc-server,
/// converted to the more convenient CourseDetails during deserialization
#[derive(Debug, Deserialize)]
struct CourseDetailsWrapper {
    pub course: CourseDetailsInner,
}

// TODO: improve
#[derive(Debug, Deserialize)]
struct CourseDetailsInner {
    #[serde(flatten)]
    pub course: Course,
    pub unlockables: Vec<String>,
    pub exercises: Vec<Exercise>,
}

/// get /api/v8/core/courses/{course_id}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(from = "CourseDetailsWrapper")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
// we never take these structs as inputs from TS so it's ok to ignore from
#[cfg_attr(feature = "ts-rs", ts(ignore_serde_attr = "from"))]
pub struct CourseDetails {
    #[serde(flatten)]
    pub course: Course,
    pub unlockables: Vec<String>,
    pub exercises: Vec<Exercise>,
}

impl From<CourseDetailsWrapper> for CourseDetails {
    fn from(value: CourseDetailsWrapper) -> Self {
        Self {
            course: value.course.course,
            unlockables: value.course.unlockables,
            exercises: value.course.exercises,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Exercise {
    pub id: u32,
    pub name: String,
    pub locked: bool,
    pub deadline_description: Option<String>,
    pub deadline: Option<String>,
    pub soft_deadline: Option<String>,
    pub soft_deadline_description: Option<String>,
    pub checksum: String,
    /// /api/v8/core/exercises/{exercise_id}/submissions
    pub return_url: String,
    /// /api/v8/core/exercises/{exercise_id}/download
    pub zip_url: String,
    pub returnable: bool,
    pub requires_review: bool,
    pub attempted: bool,
    pub completed: bool,
    pub reviewed: bool,
    pub all_review_points_given: bool,
    pub memory_limit: Option<u32>,
    pub runtime_params: Vec<String>,
    pub valgrind_strategy: Option<String>,
    pub code_review_requests_enabled: bool,
    pub run_tests_locally_action_enabled: bool,
    /// Typically null.
    pub latest_submission_url: Option<String>,
    pub latest_submission_id: Option<u32>,
    /// /api/v8/core/exercises/{exercise_id}/solution/download
    pub solution_zip_url: Option<String>,
}

/// get /api/v8/courses/{course_id}/exercises
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct CourseExercise {
    pub id: u32,
    pub available_points: Vec<ExercisePoint>,
    pub awarded_points: Vec<String>,
    pub name: String,
    pub publish_time: Option<String>,
    pub solution_visible_after: Option<String>,
    pub deadline: Option<String>,
    pub soft_deadline: Option<String>,
    pub disabled: bool,
    pub unlocked: bool,
}

/// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises
#[derive(Debug, Deserialize)]
pub struct CourseDataExercise {
    pub id: u32,
    pub available_points: Vec<ExercisePoint>,
    pub name: String,
    pub publish_time: Option<String>,
    pub solution_visible_after: Option<String>,
    pub deadline: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExercisePoint {
    pub id: u32,
    pub exercise_id: u32,
    pub name: String,
    pub requires_review: bool,
}

/// get /api/v8/courses/{course_id}/points
/// get /api/v8/courses/{course_id}/exercises/{exercise_name}/points
/// get /api/v8/courses/{course_id}/exercises/{exercise_name}/users/{user_id}/
/// get /api/v8/courses/{course_id}/exercises/{exercise_name}/users/current/points
/// get /api/v8/courses/{course_id}/users/{user_id}/points
/// get /api/v8/courses/{course_id}/users/current/points
/// get /api/v8/org/{organization_slug}/courses/{course_name}/points
/// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises/{exercise_name}/points
/// get /api/v8/org/{organization_slug}/courses/{course_name}/users/{user_id}/points
/// get /api/v8/org/{organization_slug}/courses/{course_name}/users/current/points
#[derive(Debug, Deserialize)]
pub struct CourseDataExercisePoint {
    pub awarded_point: AwardedPoint,
    pub exercise_id: u32,
}

#[derive(Debug, Deserialize)]
pub struct AwardedPoint {
    pub id: u32,
    pub course_id: u32,
    pub user_id: u32,
    pub submission_id: u32,
    pub name: String,
    pub created_at: DateTime<FixedOffset>,
}

/// get /api/v8/core/exercises/{exercise_id}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExerciseDetails {
    pub course_name: String,
    pub course_id: u32,
    pub code_review_requests_enabled: bool,
    pub run_tests_locally_action_enabled: bool,
    pub exercise_name: String,
    pub exercise_id: u32,
    pub unlocked_at: Option<String>,
    pub deadline: Option<String>,
    pub submissions: Vec<ExerciseSubmission>,
}

/// get /api/v8/core/exercises/details
#[derive(Debug, Deserialize)]
pub(crate) struct ExercisesDetailsWrapper {
    pub exercises: Vec<ExercisesDetails>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExercisesDetails {
    pub id: u32,
    pub course_name: String,
    pub exercise_name: String,
    pub checksum: String,
    pub hide_submission_results: bool,
}

/// get /api/v8/courses/{course_id}/submissions
/// get /api/v8/courses/{course_id}/users/{user_id}/submissions
/// get /api/v8/courses/{course_id}/users/current/submissions
/// get api/v8/exercises/{exercise_id}/users/{user_id}/submissions
/// get api/v8/exercises/{exercise_id}/users/current/submissions
/// get /api/v8/org/{organization_slug}/courses/{course_name}/submissions
/// get /api/v8/org/{organization_slug}/courses/{course_name}/users/{user_id}/submissions
/// get /api/v8/org/{organization_slug}/courses/{course_name}/users/current/submissions
/// get api/v8/exercises/{exercise_id}/users/{user_id}/submissions
/// get api/v8/exercises/{exercise_id}/users/current/submissions
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Submission {
    pub id: u32,
    pub user_id: u32,
    pub pretest_error: Option<String>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub created_at: DateTime<FixedOffset>,
    pub exercise_name: String,
    pub course_id: u32,
    pub processed: bool,
    pub all_tests_passed: bool,
    pub points: Option<String>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string | null"))]
    pub processing_tried_at: Option<DateTime<FixedOffset>>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string | null"))]
    pub processing_began_at: Option<DateTime<FixedOffset>>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string | null"))]
    pub processing_completed_at: Option<DateTime<FixedOffset>>,
    pub times_sent_to_sandbox: u32,
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub processing_attempts_started_at: DateTime<FixedOffset>,
    pub params_json: Option<String>,
    pub requires_review: bool,
    pub requests_review: bool,
    pub reviewed: bool,
    pub message_for_reviewer: String,
    pub newer_submission_reviewed: bool,
    pub review_dismissed: bool,
    pub paste_available: bool,
    pub message_for_paste: String,
    pub paste_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExerciseSubmission {
    pub exercise_name: String,
    pub id: u32,
    pub user_id: u32,
    pub course_id: u32,
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub created_at: DateTime<FixedOffset>,
    pub all_tests_passed: bool,
    pub points: Option<String>,
    /// /api/v8/core/submissions/{submission_id}/download
    pub submitted_zip_url: String,
    /// https://tmc.mooc.fi/paste/{paste_code}
    pub paste_url: Option<String>,
    pub processing_time: Option<u32>,
    pub reviewed: bool,
    pub requests_review: bool,
}

/// post /api/v8/core/exercises/{exercise_id}/submissions
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct NewSubmission {
    /// https://tmc.mooc.fi/api/v8/core/submissions/{submission_id}
    pub show_submission_url: String,
    /// https://tmc.mooc.fi/paste/{paste_code}
    pub paste_url: String, // use Option and serde_with::string_empty_as_none ?
    /// https://tmc.mooc.fi/submissions/{submission_id}
    pub submission_url: String,
}

/// get /api/v8/core/submission/{submission_id}
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)] // TODO: tag
pub enum SubmissionProcessingStatus {
    Processing(SubmissionProcessing),
    Finished(Box<SubmissionFinished>),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SubmissionProcessing {
    pub status: SubmissionStatus,
    pub sandbox_status: SandboxStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    Created,
    SendingToSandbox,
    ProcessingOnSandbox,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct SubmissionFinished {
    pub api_version: u32,
    pub all_tests_passed: Option<bool>,
    pub user_id: u32,
    pub login: String,
    pub course: String,
    pub exercise_name: String,
    pub status: SubmissionStatus,
    pub points: Vec<String>,
    pub valgrind: Option<String>,
    /// https://tmc.mooc.fi/submissions/{submission_id}}
    pub submission_url: String,
    /// https://tmc.mooc.fi/exercises/{exercise_id}/solution
    pub solution_url: Option<String>,
    pub submitted_at: String,
    pub processing_time: Option<u32>,
    pub reviewed: bool,
    pub requests_review: bool,
    /// https://tmc.mooc.fi/paste/{paste_code}
    pub paste_url: Option<String>,
    pub message_for_paste: Option<String>,
    pub missing_review_points: Vec<String>,
    pub test_cases: Option<Vec<TestCase>>,
    pub feedback_questions: Option<Vec<SubmissionFeedbackQuestion>>,
    /// /api/v8/core/submissions/{submission_id}/feedback
    pub feedback_answer_url: Option<String>,
    pub error: Option<String>,
    pub validations: Option<StyleValidationResult>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum SubmissionStatus {
    Processing,
    Fail,
    Ok,
    Error,
    Hidden,
}

/// post /api/v8/core/submissions/{submission_id}/feedback
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct SubmissionFeedbackResponse {
    pub api_version: u32,
    pub status: SubmissionStatus,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct TestCase {
    pub name: String,
    pub successful: bool,
    pub message: Option<String>,
    pub exception: Option<Vec<String>>,
    pub detailed_message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct SubmissionFeedbackQuestion {
    pub id: u32,
    pub question: String,
    pub kind: SubmissionFeedbackKind,
}

#[derive(Debug, PartialEq, Eq, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum SubmissionFeedbackKind {
    Text,
    IntRange { lower: u32, upper: u32 },
}

impl<'de> Deserialize<'de> for SubmissionFeedbackKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(SubmissionFeedbackKindVisitor {})
    }
}

impl Serialize for SubmissionFeedbackKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = match self {
            Self::Text => "text".to_string(),
            Self::IntRange { lower, upper } => format!("intrange[{}..{}]", lower, upper),
        };
        serializer.serialize_str(&s)
    }
}

struct SubmissionFeedbackKindVisitor {}

// parses "text" into Text, and "intrange[x..y]" into IntRange {lower: x, upper: y}
impl<'de> Visitor<'de> for SubmissionFeedbackKindVisitor {
    type Value = SubmissionFeedbackKind;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("\"text\" or \"intrange[x..y]\"")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        #[allow(clippy::unwrap_used)]
        static RANGE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r#"intrange\[(\d+)\.\.(\d+)\]"#).unwrap());

        if value == "text" {
            Ok(SubmissionFeedbackKind::Text)
        } else if let Some(captures) = RANGE.captures(value) {
            let lower = &captures[1];
            let lower = u32::from_str(lower).map_err(|e| {
                E::custom(format!(
                    "error parsing intrange lower bound {}: {}",
                    lower, e
                ))
            })?;
            let upper = &captures[2];
            let upper = u32::from_str(upper).map_err(|e| {
                E::custom(format!(
                    "error parsing intrange upper bound {}: {}",
                    upper, e
                ))
            })?;
            Ok(SubmissionFeedbackKind::IntRange { lower, upper })
        } else {
            Err(E::custom("expected \"text\" or \"intrange[x..y]\""))
        }
    }
}

/// get /api/v8/core/courses/{course_id}/reviews
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct Review {
    pub submission_id: u32,
    pub exercise_name: String,
    pub id: u32,
    pub marked_as_read: bool,
    pub reviewer_name: String,
    pub review_body: String,
    pub points: Vec<String>,
    pub points_not_awarded: Vec<String>,
    /// https://tmc.mooc.fi/submissions/{submission_id}/reviews
    pub url: String,
    /// /api/v8/core/courses/{course_id}/reviews/{review_id}
    pub update_url: String,
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub created_at: DateTime<FixedOffset>,
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub updated_at: DateTime<FixedOffset>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use tmc_langs_util::deserialize;

    fn init() {
        use log::*;
        use simple_logger::*;
        // the module levels must be set here too for some reason,
        // even though this module does not use mockito etc.
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            // mockito does some logging
            .with_module_level("mockito", LevelFilter::Warn)
            // reqwest does a lot of logging
            .with_module_level("reqwest", LevelFilter::Warn)
            // hyper does a lot of logging
            .with_module_level("hyper", LevelFilter::Warn)
            .init();
    }

    #[test]
    fn course_details_de() {
        init();

        let details = serde_json::json!(
            {
                "course": {
                    "comet_url": "c",
                    "description": "d",
                    "details_url": "du",
                    "id": 1,
                    "name": "n",
                    "reviews_url": "r",
                    "spyware_urls": [
                        "s"
                    ],
                    "title": "t",
                    "unlock_url": "u",
                    "unlockables": ["a"],
                    "exercises": []
                }
            }
        );
        assert!(deserialize::json_from_value::<CourseDetails>(details).is_ok());
    }

    #[test]
    fn feedback_kind_de() {
        init();

        let text = serde_json::json!("text");
        let text: SubmissionFeedbackKind = deserialize::json_from_value(text).unwrap();
        if let SubmissionFeedbackKind::Text = text {
        } else {
            panic!("wrong type")
        }

        let intrange = serde_json::json!("intrange[1..5]");
        let intrange: SubmissionFeedbackKind = deserialize::json_from_value(intrange).unwrap();
        if let SubmissionFeedbackKind::IntRange { lower: 1, upper: 5 } = intrange {
        } else {
            panic!("wrong type")
        }
    }

    #[test]
    fn feedback_kind_se() {
        init();
        use serde_json::Value;

        let text = SubmissionFeedbackKind::Text;
        let text = serde_json::to_value(&text).unwrap();
        assert_eq!(text, Value::String("text".to_string()));

        let range = SubmissionFeedbackKind::IntRange { lower: 1, upper: 5 };
        let range = serde_json::to_value(&range).unwrap();
        assert_eq!(range, Value::String("intrange[1..5]".to_string()));
    }
}
