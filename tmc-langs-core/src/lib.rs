//! Used to communicate with the TMC server. See the TmcCore struct for more details.
//!
//! ```rust,no_run
//! use tmc_langs_core::TmcCore;
//!
//! let mut core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string()).unwrap();
//! core.authenticate("client_name", "email".to_string(), "password".to_string());
//! let organizations = core.get_organizations();
//! ```
//!

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
pub use tmc_langs_util::{Language, RunResult, Strategy, ValidationResult};
