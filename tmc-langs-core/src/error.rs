//! The core error type.

use crate::response;
use reqwest::StatusCode;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

pub(crate) type Result<T> = std::result::Result<T, CoreError>;
type TokenError = oauth2::RequestTokenError<
    CoreError,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Failed to create temporary file: {0}")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to create file at {0}: {1}")]
    FileCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to open file at {0}: {1}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to write to file at {0}: {1}")]
    Write(PathBuf, #[source] std::io::Error),
    #[error("HTTP error for {0}: {1}")]
    HttpStatus(Url, StatusCode),
    #[error("OAuth2 password exchange error: {0}")]
    Token(Box<TokenError>),
    #[error("OAuth2 unexpected token response {1}: {0}")]
    TokenParse(#[source] serde_json::error::Error, String),
    #[error("Already authenticated")]
    AlreadyAuthenticated,
    #[error("Authentication required")]
    AuthRequired,
    #[error("Failed to find cache directory")]
    CacheDir,

    #[error(transparent)]
    InvalidMethod(#[from] http::method::InvalidMethod),
    #[error(transparent)]
    InvalidHeaderName(#[from] http::header::InvalidHeaderName),
    #[error(transparent)]
    InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
    #[error(transparent)]
    InvalidStatusCode(#[from] http1::status::InvalidStatusCode),
    #[error(transparent)]
    InvalidHeaderName1(#[from] http1::header::InvalidHeaderName),
    #[error(transparent)]
    InvalidHeaderValue1(#[from] http1::header::InvalidHeaderValue),
    #[error(transparent)]
    TmcLangs(#[from] tmc_langs_util::Error),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Parse(#[from] url::ParseError),
    #[error(transparent)]
    Parse1(#[from] url1::ParseError),
    #[error(transparent)]
    Response(#[from] response::ResponseError),
    #[error(transparent)]
    ResponseErrors(#[from] response::ResponseErrors),
    #[error(transparent)]
    SystemTime(#[from] std::time::SystemTimeError),
}

impl From<TokenError> for CoreError {
    fn from(err: TokenError) -> Self {
        Self::Token(Box::new(err))
    }
}
