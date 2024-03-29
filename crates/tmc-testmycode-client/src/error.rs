//! The client error type.

use reqwest::{Method, StatusCode};
use thiserror::Error;
use tmc_langs_util::{FileError, JsonError};
use url::Url;

type TokenError = oauth2::RequestTokenError<
    oauth2::reqwest::Error<reqwest::Error>,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

pub type TestMyCodeClientResult<T> = Result<T, TestMyCodeClientError>;

/// The main error type for tmc-testmycode-client.
#[derive(Debug, Error)]
pub enum TestMyCodeClientError {
    #[error("HTTP error {status} for {url}: {error}. Obsolete client: {obsolete_client}")]
    HttpError {
        url: Url,
        status: StatusCode,
        error: String,
        obsolete_client: bool,
    },
    #[error("Connection error trying to {0} {1}")]
    ConnectionError(Method, Url, #[source] reqwest::Error),
    #[error("OAuth2 password exchange error")]
    Token(#[source] Box<TokenError>),
    #[error("Failed to parse as URL: {0}")]
    UrlParse(String, #[source] url::ParseError),
    #[error("Failed to write response")]
    HttpWriteResponse(#[source] reqwest::Error),
    #[error("Failed to read response")]
    HttpReadResponse(#[source] reqwest::Error),
    #[error("Failed to deserialize response from {0} as JSON")]
    HttpJsonResponse(Url, #[source] JsonError),

    #[error("Already authenticated")]
    AlreadyAuthenticated,
    #[error("Authentication required")]
    NotAuthenticated,

    #[error(transparent)]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    FileError(#[from] FileError),
    #[error(transparent)]
    Plugin(#[from] tmc_langs_plugins::PluginError),
}

impl From<TokenError> for TestMyCodeClientError {
    fn from(err: TokenError) -> Self {
        Self::Token(Box::new(err))
    }
}
