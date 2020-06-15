//! Used to communicate with the TMC server.

mod error;
mod request;
mod response;
mod tmc_core;

pub use error::CoreError;
pub use request::FeedbackAnswer;
pub use response::{
    Course, CourseDetails, CourseExercise, ExerciseDetails, NewSubmission, NuCourse,
    NuCourseExercise, NuExercisePoint, Organization, Review, Submission,
    SubmissionFeedbackResponse, User,
};
pub use tmc_core::TmcCore;
