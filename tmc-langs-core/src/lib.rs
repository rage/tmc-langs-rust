//! Used to communicate with the TMC server.

mod error;
mod request;
mod response;
mod tmc_core;

pub use error::CoreError;
pub use request::FeedbackAnswer;
pub use response::{
    Course, CourseData, CourseDataExercise, CourseDataExercisePoint, CourseDetails, CourseExercise,
    Exercise, ExerciseDetails, NewSubmission, Organization, Review, Submission,
    SubmissionFeedbackResponse, SubmissionFinished, SubmissionProcessingStatus, SubmissionStatus,
    User,
};
pub use tmc_core::TmcCore;
pub use tmc_langs_util::{Language, RunResult, ValidationResult};
