use crate::CoreError;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Response<T> {
    Ok(T),
    Err(ResponseError),
}

impl<T> Response<T> {
    pub fn to_result(self) -> Result<T, CoreError> {
        match self {
            Self::Ok(t) => Ok(t),
            Self::Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Error, Deserialize)]
#[error("Response contained an error: {errors:#?}")]
pub struct ResponseError {
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub application_id: String,
    pub secret: String,
}

#[derive(Debug, Deserialize)]
pub struct Organization {
    pub name: String,
    pub information: String,
    pub slug: String,
    pub logo_path: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
pub struct Course {
    id: usize,
    name: String,
    title: String,
    description: String,
    details_url: String,
    unlock_url: String,
    reviews_url: String,
    comet_url: String,
    spyware_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CourseDetailsWrapper {
    pub course: CourseDetails,
}

#[derive(Debug, Deserialize)]
pub struct CourseDetails {
    #[serde(flatten)]
    pub course: Course,
    pub unlockables: Vec<String>,
    pub exercises: Vec<Exercise>,
}

#[derive(Debug, Deserialize)]
pub struct Exercise {
    id: usize,
    name: String,
    locked: bool,
    deadline_description: Option<String>,
    deadline: Option<String>,
    soft_deadline: Option<String>,
    soft_deadline_description: Option<String>,
    checksum: String,
    return_url: String,
    zip_url: String,
    returnable: bool,
    requires_review: bool,
    attempted: bool,
    completed: bool,
    reviewed: bool,
    all_review_points_given: bool,
    memory_limit: Option<usize>,
    runtime_params: Vec<String>,
    valgrind_strategy: String,
    code_review_requests_enabled: bool,
    run_tests_locally_action_enabled: bool,
    latest_submission_url: Option<String>,
    latest_submission_id: Option<usize>,
    solution_zip_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CourseExercise {
    id: usize,
    available_points: Vec<ExercisePoint>,
    awarded_points: Vec<String>,
    name: String,
    publish_time: Option<String>,
    solution_visible_after: Option<String>,
    deadline: Option<String>,
    soft_deadline: Option<String>,
    disabled: bool,
    unlocked: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExercisePoint {
    id: usize,
    exercise_id: usize,
    name: String,
    requires_review: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExerciseDetails {
    course_name: String,
    course_id: usize,
    code_review_requests_enabled: bool,
    run_tests_locally_action_enabled: bool,
    exercise_name: String,
    exercise_id: usize,
    unlocked_at: Option<String>,
    deadline: Option<String>,
    // submissions: Vec<Submission>, // not used?
}

#[derive(Debug, Deserialize)]
pub struct Submission {}
