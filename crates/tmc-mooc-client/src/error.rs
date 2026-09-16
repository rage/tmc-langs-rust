//! Error type for the crate.

use reqwest::{Method, StatusCode, Url};
use std::error::Error;
use thiserror::Error;
use uuid::Uuid;

pub type MoocClientResult<T> = Result<T, Box<MoocClientError>>;

/// Replaces every query-string value, keeping the keys and the rest of the URL.
///
/// A file URL from this API carries a signed one-hour download capability as a query
/// parameter, so any URL that reaches a log or an error message has to shed its query
/// values. Redaction is positional rather than keyed on the parameter's name, which is
/// defined in the backend repo and would fail open here the day it changes; nothing in this
/// crate builds a query string of its own, so no value is worth preserving. Takes `&str` so
/// the same rule covers [`MoocClientError::UrlParse`], whose input did not parse as a URL.
pub(crate) fn redact_query(url: &str) -> String {
    const REDACTED: &str = "<redacted>";
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let mut redacted = String::with_capacity(base.len() + query.len());
    redacted.push_str(base);
    redacted.push('?');
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            redacted.push('&');
        }
        match pair.split_once('=') {
            Some((key, _)) => {
                redacted.push_str(key);
                redacted.push('=');
                redacted.push_str(REDACTED);
            }
            // A valueless parameter is itself the value.
            None => redacted.push_str(REDACTED),
        }
    }
    redacted
}

#[derive(Debug, Error)]
pub enum MoocClientError {
    #[error(
        "HTTP error {status} for {}: {error}. Obsolete client: {obsolete_client}.",
        redact_query(url.as_str())
    )]
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
    #[error("Connection error trying to {} {}", .0, redact_query(.1.as_str()))]
    ConnectionError(Method, Url, #[source] reqwest::Error),
    #[error("Failed to parse as URL: {}", redact_query(.0))]
    UrlParse(String, #[source] url::ParseError),
    #[error(
        "Refusing to use an insecure URL scheme for {}: courses.mooc.fi must be reached over \
         https so the bearer token is never sent in plaintext",
        redact_query(url.as_str())
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
    #[error(
        "Failed to read {method} response body from {}: {error}.",
        redact_query(url.as_str())
    )]
    ReadingResponseBody {
        method: Method,
        url: Url,
        error: Box<dyn Error + Send + Sync>,
    },
    #[error("Failed to deserialize response body from {}: {error}.", redact_query(url.as_str()))]
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
}

#[cfg(test)]
mod test {
    use super::redact_query;

    #[test]
    fn redact_query_leaves_a_url_without_a_query_alone() {
        assert_eq!(
            redact_query("https://courses.mooc.fi/api/v0/files/claimed/an-id"),
            "https://courses.mooc.fi/api/v0/files/claimed/an-id"
        );
    }

    #[test]
    fn redact_query_replaces_every_value_and_keeps_every_key() {
        // Positional, so a renamed claim parameter is still caught.
        assert_eq!(
            redact_query("https://host/f?download-claim=secret&renamed-later=secret&bare"),
            "https://host/f?download-claim=<redacted>&renamed-later=<redacted>&<redacted>"
        );
    }
}
