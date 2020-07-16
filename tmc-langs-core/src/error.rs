//! The core error type.

use crate::response;
use reqwest::StatusCode;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

pub(crate) type Result<T> = std::result::Result<T, CoreError>;
type TokenError = oauth2::RequestTokenError<
    oauth2::reqwest::Error<reqwest::Error>,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

#[derive(Debug, Error)]
pub enum CoreError {
    // file IO
    #[error("Failed to create temporary file")]
    TempFile(#[source] std::io::Error),
    #[error("Failed to create file at {0}")]
    FileCreate(PathBuf, #[source] std::io::Error),
    #[error("Failed to open file at {0}")]
    FileOpen(PathBuf, #[source] std::io::Error),
    #[error("Failed to write to file at {0}")]
    FileWrite(PathBuf, #[source] std::io::Error),
    #[error("Failed to remove directory at {0}")]
    DirRemove(PathBuf, #[source] std::io::Error),

    // network
    #[error("HTTP error {1} for {0}: {2}")]
    HttpStatus(Url, StatusCode, String),
    #[error("OAuth2 password exchange error")]
    Token(#[source] Box<TokenError>),
    #[error("OAuth2 unexpected token response: {0}")]
    TokenParse(String, #[source] serde_json::error::Error),
    #[error("Failed to parse as URL: {0}")]
    UrlParse(String, #[source] url::ParseError),
    #[error("Failed to GET {0}")]
    HttpGet(Url, #[source] reqwest::Error),
    #[error("Failed to POST {0}")]
    HttpPost(Url, #[source] reqwest::Error),
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
    TmcLangs(#[from] tmc_langs_util::TmcError),
    #[error(transparent)]
    Response(#[from] response::ResponseError),
    #[error(transparent)]
    ResponseErrors(#[from] response::ResponseErrors),
}

impl From<TokenError> for CoreError {
    fn from(err: TokenError) -> Self {
        Self::Token(Box::new(err))
    }
}
