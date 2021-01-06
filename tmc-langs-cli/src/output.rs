//! Output format

use schemars::JsonSchema;
use serde::Serialize;
use std::path::PathBuf;
use tmc_client::{
    ClientUpdateData, Course, CourseData, CourseDetails, CourseExercise, ExerciseDetails,
    NewSubmission, Organization, Review, RunResult, StyleValidationResult, Submission,
    SubmissionFeedbackResponse, SubmissionFinished, Token, UpdateResult,
};
use tmc_langs_util::{
    progress_reporter::StatusUpdate,
    task_executor::{RefreshData, RefreshUpdateData},
    ExerciseDesc, ExercisePackagingConfiguration,
};

use crate::config::{ConfigValue, TmcConfig};

/// The format for all messages written to stdout by the CLI
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "output-kind")]
pub enum Output {
    OutputData(OutputData),
    StatusUpdate(StatusUpdateData),
    Warnings(Warnings),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct OutputData {
    pub status: Status,
    pub message: Option<String>,
    pub result: OutputResult,
    pub percent_done: f64,
    pub data: Option<Data>,
}

#[derive(Debug, Serialize)]
pub enum Data {
    Error { kind: Kind, trace: Vec<String> },
    Validation(StyleValidationResult),
    FreeDiskSpace(u64),
    AvailablePoints(Vec<String>),
    Exercises(Vec<PathBuf>),
    ExercisePackagingConfiguration(ExercisePackagingConfiguration),
    LocalExercises(Vec<LocalExercise>),
    RefreshResult(RefreshData),
    RunResult(RunResult),
    ExerciseDesc(ExerciseDesc),
    UpdatedExercises(Vec<UpdatedExercise>),
    DownloadOrUpdateCourseExercisesResult(DownloadOrUpdateCourseExercisesResult),
    CombinedCourseData(Box<CombinedCourseData>),
    CourseDetails(CourseDetails),
    CourseExercises(Vec<CourseExercise>),
    CourseData(CourseData),
    Courses(Vec<Course>),
    ExerciseDetails(ExerciseDetails),
    Submissions(Vec<Submission>),
    UpdateResult(UpdateResult),
    Organization(Organization),
    Organizations(Vec<Organization>),
    Reviews(Vec<Review>),
    Token(Token),
    NewSubmission(NewSubmission),
    StyleValidationResult(StyleValidationResult),
    SubmissionFeedbackResponse(SubmissionFeedbackResponse),
    SubmissionFinished(SubmissionFinished),
    ConfigValue(ConfigValue<'static>),
    TmcConfig(TmcConfig),
}

#[derive(Debug, Serialize)]
pub enum StatusUpdateData {
    RefreshUpdateData(StatusUpdate<RefreshUpdateData>),
    ClientUpdateData(StatusUpdate<ClientUpdateData>),
    None(StatusUpdate<()>),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// The command was ran without fatal errors
    Finished,
    /// An unexpected issue occurred during the command
    Crashed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputResult {
    LoggedIn,
    LoggedOut,
    NotLoggedIn,
    Error,
    SentData,
    RetrievedData,
    ExecutedCommand,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// For all other errors
    Generic,
    /// 403 from server
    Forbidden,
    /// Not logged in, detected either by no token or 401 from server
    NotLoggedIn,
    /// Failed to connect to the TMC server, likely due to no internet connection
    ConnectionError,
    /// Client out of date
    ObsoleteClient,
    /// Invalid token
    InvalidToken,
    /// Failed to download some or all exercises
    FailedExerciseDownload {
        completed: Vec<usize>,
        skipped: Vec<usize>,
        failed: Vec<(usize, Vec<String>)>,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CombinedCourseData {
    pub details: CourseDetails,
    pub exercises: Vec<CourseExercise>,
    pub settings: CourseData,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadOrUpdateCourseExercisesResult {
    pub downloaded: Vec<DownloadOrUpdateCourseExercise>,
    pub skipped: Vec<DownloadOrUpdateCourseExercise>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct DownloadOrUpdateCourseExercise {
    pub course_slug: String,
    pub exercise_slug: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct LocalExercise {
    pub exercise_slug: String,
    pub exercise_path: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct UpdatedExercise {
    pub id: usize,
}

#[derive(Debug, Serialize)]
pub struct DownloadTarget {
    pub id: usize,
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct Warnings {
    warnings: Vec<String>,
}

impl Warnings {
    pub fn from_error_list(warnings: &[anyhow::Error]) -> Self {
        Self {
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
        }
    }
}
