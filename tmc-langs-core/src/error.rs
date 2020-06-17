use crate::response;
use reqwest::StatusCode;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

pub(crate) type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Failed to create temporary file: {0}")]
    TempFile(std::io::Error),
    #[error("Failed to create file at {0}: {1}")]
    FileCreate(PathBuf, std::io::Error),
    #[error("Failed to open file at {0}: {1}")]
    FileOpen(PathBuf, std::io::Error),
    #[error("Failed to write to file at {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("HTTP error for {0}: {1}")]
    HttpStatus(Url, StatusCode),
    #[error("OAuth2 password exchange error: {0}")]
    Token(oauth2::RequestTokenError<oauth2::basic::BasicErrorResponseType>),
    #[error("Already authenticated")]
    AlreadyAuthenticated,
    #[error("Authentication required")]
    AuthRequired,
    #[error("Failed to find cache directory")]
    CacheDir,

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
