//! Error type for the crate.

use reqwest::{Method, StatusCode, Url};
use std::error::Error;
use thiserror::Error;
use uuid::Uuid;

pub type MoocClientResult<T> = Result<T, Box<MoocClientError>>;

#[derive(Debug, Error)]
pub enum MoocClientError {
    #[error("HTTP error {status} for {url}: {error}. Obsolete client: {obsolete_client}.")]
    HttpError {
        url: Url,
        status: StatusCode,
        error: String,
        obsolete_client: bool,
        /// `message_key` parsed off the backend's `ApiErrorResponse` body, if the
        /// body was that shape. Lets the CLI map controlled errors (e.g.
        /// `not_enrolled`) to a typed `Kind` while `error` keeps the raw body for
        /// human-readable messages.
        message_key: Option<String>,
    },
    #[error("Connection error trying to {0} {1}")]
    ConnectionError(Method, Url, #[source] reqwest::Error),
    #[error("Failed to parse as URL: {0}")]
    UrlParse(String, #[source] url::ParseError),
    #[error(
        "Refusing to use an insecure URL scheme for {url}: courses.mooc.fi must be reached over \
         https so the bearer token is never sent in plaintext"
    )]
    InsecureScheme { url: Url },
    #[error("Authentication required")]
    NotAuthenticated,
    #[error("The device authorization request expired before it was approved")]
    DeviceCodeExpired,
    #[error("The device authorization request was denied")]
    DeviceAccessDenied,
    /// The token endpoint rejected a refresh grant with an OAuth error (a 400
    /// with an `error` code, e.g. `invalid_grant`), meaning the refresh token is
    /// no longer valid — revoked or expired. This is a *permanent* failure:
    /// retrying will not help, so stored credentials should be discarded. It is
    /// deliberately distinct from [`Self::HttpError`] / [`Self::ConnectionError`],
    /// which are transient (5xx, timeouts, dropped connections) and must NOT
    /// cause credentials to be deleted.
    #[error("The authorization server rejected the refresh token: {error}")]
    RefreshTokenRejected { error: String },
    #[error("Failed to attach file to submission form: {error}")]
    AttachFileToForm { error: Box<dyn Error + Send + Sync> },
    #[error("Failed to send {method} request to {url}: {error}.")]
    SendingRequest {
        method: Method,
        url: Url,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("Failed to read {method} response body from {url}: {error}.")]
    ReadingResponseBody {
        method: Method,
        url: Url,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("Failed to deserialize response body from {url}: {error}.")]
    DeserializingResponse {
        url: Url,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error(transparent)]
    JsonError(#[from] tmc_langs_util::JsonError),
    #[error(
        "Exercise {exercise_id} has no downloadable editor task; browser exercises \
         have no project archive to download"
    )]
    NoDownloadableExerciseTask { exercise_id: Uuid },
    #[error(
        "Exercise {exercise_id} has no submittable editor task; browser exercises \
         are solved in-browser and cannot be submitted from a native client"
    )]
    NoSubmittableExerciseTask { exercise_id: Uuid },
    /// Only ever raised for more than one file; no files is a legitimate outcome
    /// (see [`crate::MoocClient::download_submission_archive_url`]).
    #[error(
        "Submission {submission_id} consists of {count} files, not the single project \
         archive an editor submission is made of, so it cannot be restored"
    )]
    UnexpectedSubmissionFileCount { submission_id: Uuid, count: usize },
}
