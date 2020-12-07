//! Used to communicate with the TMC server. See the TmcClient struct for more details.
//!
//! ```rust,no_run
//! use tmc_client::TmcClient;
//!
//! let mut client = TmcClient::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
//! client.authenticate("client_name", "email".to_string(), "password".to_string());
//! let organizations = client.get_organizations();
//! ```
//!

mod error;
mod request;
mod response;
mod tmc_client;

pub use self::error::ClientError;
pub use self::request::FeedbackAnswer;
pub use self::response::{
    Course, CourseData, CourseDataExercise, CourseDataExercisePoint, CourseDetails, CourseExercise,
    Exercise, ExerciseDetails, ExercisesDetails, NewSubmission, Organization, Review, Submission,
    SubmissionFeedbackResponse, SubmissionFinished, SubmissionProcessingStatus, SubmissionStatus,
    UpdateResult, User,
};
pub use self::tmc_client::{ClientUpdateData, TmcClient, Token};
pub use oauth2;
pub use tmc_langs_util::{Language, RunResult, StyleValidationResult, StyleValidationStrategy};
