//! Error type for the crate.

use reqwest::{Method, StatusCode};
use std::error::Error;
use thiserror::Error;

pub type MoocClientResult<T> = Result<T, MoocClientError>;

#[derive(Debug, Error)]
pub enum MoocClientError {
    #[error("Authentication required")]
    NotAuthenticated,
    #[error("Failed to attach file to submission form: {error}")]
    AttachFileToForm { error: Box<dyn Error + Send + Sync> },
    #[error("Failed to send {method} request to {url}: {error}.")]
    SendingRequest {
        method: Method,
        url: String,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("Failed to read {method} response body from {url}: {error}.")]
    ReadingResponseBody {
        method: Method,
        url: String,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("Failed to deserialize response body from {url}: {error}.")]
    DeserializingResponse {
        url: String,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("HTTP error {status} for {url}: {error}. Obsolete client: {obsolete_client}.")]
    ErrorResponseFromServer {
        url: String,
        status: StatusCode,
        error: String,
        obsolete_client: bool,
    },
    #[error(transparent)]
    JsonError(#[from] tmc_langs_util::JsonError),
}
