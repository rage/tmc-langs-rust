//! Used to communicate with the TMC server. See the TmcCore struct for more details.
//!
//! ```rust,no_run
//! use tmc_langs_core::TmcCore;
//!
//! let mut core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
//! core.authenticate("client_name", "email".to_string(), "password".to_string());
//! let organizations = core.get_organizations();
//! ```
//!

mod error;
mod request;
mod response;
mod tmc_core;

pub use error::CoreError;
pub use oauth2;
pub use request::FeedbackAnswer;
pub use response::{
    Course, CourseData, CourseDataExercise, CourseDataExercisePoint, CourseDetails, CourseExercise,
    Exercise, ExerciseChecksums, ExerciseDetails, NewSubmission, Organization, Review, Submission,
    SubmissionFeedbackResponse, SubmissionFinished, SubmissionProcessingStatus, SubmissionStatus,
    UpdateResult, User,
};
pub use tmc_core::{CoreUpdateData, TmcCore, Token};
pub use tmc_langs_util::{Language, RunResult, Strategy, ValidationResult};
