//! The core error type.

use crate::response;
use reqwest::{Method, StatusCode};
use std::path::PathBuf;
use thiserror::Error;
use tmc_langs_util::FileIo;
use url::Url;

type TokenError = oauth2::RequestTokenError<
    oauth2::reqwest::Error<reqwest::Error>,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

#[derive(Debug, Error)]
pub enum CoreError {
    // file IO
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),

    // network
    #[error("HTTP error {1} for {0}: {2}")]
    HttpError(Url, StatusCode, String),
    #[error("Connection error trying to {0} {1}")]
    ConnectionError(Method, Url, #[source] reqwest::Error),
    #[error("OAuth2 password exchange error")]
    Token(#[source] Box<TokenError>),
    #[error("OAuth2 unexpected token response: {0}")]
    TokenParse(String, #[source] serde_json::error::Error),
    #[error("Failed to parse as URL: {0}")]
    UrlParse(String, #[source] url::ParseError),
    #[error("Failed to write response to {0}")]
    HttpWriteResponse(PathBuf, #[source] reqwest::Error),
    #[error("Failed to deserialize response as JSON")]
    HttpJsonResponse(#[source] reqwest::Error),

    #[error("Already authenticated")]
    AlreadyAuthenticated,
    #[error("Authentication required")]
    AuthRequired,
    #[error("Failed to find cache directory")]
    CacheDir,

    #[error(transparent)]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error(transparent)]
    Response(#[from] response::ResponseError),
    #[error(transparent)]
    ResponseErrors(#[from] response::ResponseErrors),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error("File IO error")]
    FileIo(#[from] FileIo),
    #[error(transparent)]
    Tmc(#[from] tmc_langs_util::error::UtilError),
}

impl From<TokenError> for CoreError {
    fn from(err: TokenError) -> Self {
        Self::Token(Box::new(err))
    }
}
