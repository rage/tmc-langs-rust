//! Error type for the crate.

use reqwest::{Method, StatusCode, Url};
use std::error::Error;
use thiserror::Error;
use tmc_langs_util::http_error::{describe_http_error, quotable_body};
use uuid::Uuid;

pub type MoocClientResult<T> = Result<T, Box<MoocClientError>>;

/// Replaces every query-string value, keeping the keys and the rest of the URL.
///
/// File URLs carry a signed download capability in the query, so URLs must be redacted
/// before reaching logs or errors. Positional rather than keyed on the parameter name,
/// which the backend owns and could change, failing open. Takes `&str` so it also covers
/// [`MoocClientError::UrlParse`], whose input did not parse as a URL.
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

/// What a backend's error body says to a person: an `ApiErrorResponse` `message` or an OAuth
/// `error_description`/`error`, else the body itself when it is fit to quote.
pub(crate) fn response_detail(body: &str) -> Option<String> {
    if let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(body) {
        let detail = ["message", "error_description", "error"]
            .into_iter()
            .filter_map(|key| fields.get(key)?.as_str())
            .map(str::trim)
            .find(|text| !text.is_empty());
        if let Some(detail) = detail {
            return Some(detail.to_string());
        }
    }
    quotable_body(body)
}

/// What an error response says besides its status and body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseMeta {
    /// The `request-id` courses.mooc.fi stamps on every response and logs the request under.
    pub request_id: Option<String>,
    /// From `Retry-After`, in seconds; only the delta-seconds form is read.
    pub retry_after_secs: Option<u64>,
}

impl ResponseMeta {
    pub(crate) fn of(headers: &reqwest::header::HeaderMap) -> Self {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        Self {
            request_id: header("request-id").map(str::to_string),
            retry_after_secs: header("retry-after").and_then(|value| value.trim().parse().ok()),
        }
    }
}

/// Why a request got no response at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionFailure {
    /// The server did not answer in time.
    Timeout,
    /// TLS failed, typically an untrusted certificate (an intercepting proxy or security
    /// product) or a wrong system clock.
    Certificate,
    /// A proxy refused the connection or asked for credentials.
    Proxy,
    /// Anything else: no network, DNS failure, connection refused.
    Unreachable,
}

impl ConnectionFailure {
    pub fn of(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            return Self::Timeout;
        }
        let mut causes = Vec::new();
        let mut source: Option<&(dyn Error + 'static)> = error.source();
        while let Some(cause) = source {
            causes.push(cause.to_string().to_lowercase());
            source = cause.source();
        }
        Self::from_causes(&causes)
    }

    // Split out because a `reqwest::Error` cannot be built in a test.
    fn from_causes(causes: &[String]) -> Self {
        let mentions = |needles: &[&str]| {
            causes
                .iter()
                .any(|cause| needles.iter().any(|needle| cause.contains(needle)))
        };
        if mentions(&["certificate", "tls", "ssl", "handshake"]) {
            Self::Certificate
        } else if mentions(&["proxy", "407"]) {
            Self::Proxy
        } else if mentions(&["timed out", "timeout"]) {
            Self::Timeout
        } else {
            Self::Unreachable
        }
    }
}

#[derive(Debug, Error)]
pub enum MoocClientError {
    #[error(
        "{}",
        describe_http_error(status, &redact_query(url.as_str()), response_detail(error).as_deref(), *obsolete_client)
    )]
    HttpError {
        url: Url,
        status: StatusCode,
        /// The raw response body.
        error: String,
        obsolete_client: bool,
        /// `message_key` of the backend's `ApiErrorResponse` body, if it had that shape;
        /// lets the CLI map controlled errors (e.g. `not_enrolled`) to a typed `Kind`.
        message_key: Option<String>,
        meta: ResponseMeta,
    },
    /// A file download answered 404: the exercise's or submission's files are gone from the
    /// backend's file store.
    #[error("The exercise's files are missing on courses.mooc.fi ({})", redact_query(url.as_str()))]
    FileNotFound { url: Url },
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
    /// A 401 from the resource API: the token is missing, expired or revoked.
    #[error("Authentication required{}", detail.as_ref().map(|detail| format!(": {detail}")).unwrap_or_default())]
    NotAuthenticated {
        detail: Option<String>,
        meta: ResponseMeta,
    },
    #[error("The device authorization request expired before it was approved")]
    DeviceCodeExpired,
    #[error("The device authorization request was denied")]
    DeviceAccessDenied,
    /// The token endpoint rejected a refresh grant with an OAuth error (a 400 with an
    /// `error` code, e.g. `invalid_grant`): the refresh token is revoked or expired.
    /// Permanent, so stored credentials should be discarded, unlike the transient
    /// [`Self::HttpError`] / [`Self::ConnectionError`], which must not delete them.
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
    #[error(
        "Exercise {exercise_id} has more than one editor task; answer it in the course material"
    )]
    SeveralEditorTasks { exercise_id: Uuid },
}

impl MoocClientError {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::HttpError { meta, .. } | Self::NotAuthenticated { meta, .. } => {
                meta.request_id.as_deref()
            }
            _ => None,
        }
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::HttpError { meta, .. } => meta.retry_after_secs,
            _ => None,
        }
    }

    pub fn connection_failure(&self) -> Option<ConnectionFailure> {
        match self {
            Self::ConnectionError(_, _, error) => Some(ConnectionFailure::of(error)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::{ConnectionFailure, redact_query};

    #[test]
    fn connection_failures_are_told_apart_by_their_cause() {
        let of = |cause: &str| ConnectionFailure::from_causes(&[cause.to_lowercase()]);
        assert_eq!(
            of("invalid peer certificate: UnknownIssuer"),
            ConnectionFailure::Certificate
        );
        assert_eq!(
            of("unsuccessful tunnel: 407 Proxy Authentication Required"),
            ConnectionFailure::Proxy
        );
        assert_eq!(of("operation timed out"), ConnectionFailure::Timeout);
        assert_eq!(
            of("dns error: failed to lookup address"),
            ConnectionFailure::Unreachable
        );
    }

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
