//! OAuth2 Device Authorization Grant (RFC 8628) and refresh-token flows for the
//! Courses MOOC backend.
//!
//! Hand-rolled on `reqwest` instead of the `oauth2` crate's device-flow helper,
//! which blocks until approval: it cannot hand the caller the verification URL
//! before polling, nor expose per-poll `slow_down` / `authorization_pending`
//! for a cancellable loop. Only the crate's [`api::Token`] type is reused.
//!
//! Endpoints (`POST .../device_authorization`, `POST .../token`) are mounted at
//! `/api/v0/main-frontend/oauth` in the backend.

use crate::{CLIENT_VERSION_HEADER, error::MoocClientError, error::MoocClientResult};
use exercise_services_api as api;
use oauth2::{RefreshToken, TokenResponse};
use reqwest::{
    Method, StatusCode,
    blocking::Client,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const FORM_CONTENT_TYPE: HeaderValue =
    HeaderValue::from_static("application/x-www-form-urlencoded");

/// Overall timeout for auth-flow HTTP requests.
///
/// The refresh path holds the refresh mutex and the credentials file lock across
/// the call (`tmc-langs` `mooc_credentials::refresh_locked`), so a hung connection
/// would block every CLI process. Public so that caller can assert its lock-wait
/// budget exceeds this.
pub const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Blocking HTTP client for the auth flows, with [`AUTH_REQUEST_TIMEOUT`].
/// Panics if the client cannot be built, like `reqwest::blocking::Client::new()`.
fn auth_client() -> Client {
    Client::builder()
        .timeout(AUTH_REQUEST_TIMEOUT)
        .build()
        .expect("failed to build auth HTTP client")
}

/// Encodes `application/x-www-form-urlencoded` bodies; `reqwest` is built
/// without `RequestBuilder::form`.
fn urlencode(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        serializer.append_pair(k, v);
    }
    serializer.finish()
}

/// Default public (native) client id. Hardcoded per the shared auth contract;
/// overridable via `TMC_LANGS_MOOC_CLIENT_ID` for tests and local development.
pub const DEFAULT_CLIENT_ID: &str = "tmc-vscode";

/// Scope requested at device authorization.
pub const DEVICE_SCOPE: &str = "exercise-services";

/// RFC 8628 device-code grant type URN used on the `/token` endpoint.
pub const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 8628 §3.5 default polling interval (seconds) when the authorization
/// server does not return one.
pub const DEFAULT_POLL_INTERVAL_SECS: u32 = 5;

/// The authorization server's response to a device authorization request
/// (RFC 8628 §3.2).
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthorizationResponse {
    /// The device verification code, polled on the token endpoint.
    pub device_code: String,
    /// The end-user code shown to the user to enter on the verification page.
    pub user_code: String,
    /// The URL the user visits to enter the `user_code`.
    pub verification_uri: String,
    /// A URL that already includes the `user_code`, if the server provides one.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Lifetime in seconds of the `device_code` and `user_code`.
    pub expires_in: u32,
    /// Minimum interval in seconds between token-endpoint polls.
    #[serde(default = "default_interval")]
    pub interval: u32,
}

fn default_interval() -> u32 {
    DEFAULT_POLL_INTERVAL_SECS
}

/// The result of a single token-endpoint poll during the device flow.
#[derive(Debug)]
pub enum DeviceTokenPoll {
    /// `authorization_pending`: the user has not yet approved; keep polling.
    Pending,
    /// `slow_down`: poll less frequently (interval must be increased by 5s).
    SlowDown,
    /// The grant was approved; the token pair was issued.
    Authorized(Box<api::Token>),
}

/// Requests a device + user code from the authorization server (RFC 8628 §3.1).
pub fn device_authorization(
    root_url: &Url,
    client_id: &str,
) -> MoocClientResult<DeviceAuthorizationResponse> {
    let url = oauth_url(root_url, "device_authorization")?;
    let client = auth_client();
    let body = urlencode(&[("client_id", client_id), ("scope", DEVICE_SCOPE)]);
    let response = client
        .post(url.clone())
        .header(CLIENT_VERSION_HEADER, env!("CARGO_PKG_VERSION"))
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(body)
        .send()
        .map_err(|e| {
            Box::new(MoocClientError::ConnectionError(
                Method::POST,
                url.clone(),
                e.without_url(),
            ))
        })?;

    let status = response.status();
    if status.is_success() {
        let bytes = read_bytes(response, &url)?;
        let parsed = serde_json::from_slice(&bytes).map_err(|e| {
            Box::new(MoocClientError::DeserializingResponse {
                url: url.clone(),
                error: Box::new(e),
            })
        })?;
        Ok(parsed)
    } else {
        Err(http_error(response, url))
    }
}

/// Polls the token endpoint once with the device-code grant (RFC 8628 §3.4).
///
/// `authorization_pending` and `slow_down` map to [`DeviceTokenPoll`] variants;
/// `expired_token` and `access_denied` map to [`MoocClientError::DeviceCodeExpired`]
/// and [`MoocClientError::DeviceAccessDenied`]. Other failures are plain errors.
pub fn poll_device_token(
    root_url: &Url,
    client_id: &str,
    device_code: &str,
) -> MoocClientResult<DeviceTokenPoll> {
    let url = oauth_url(root_url, "token")?;
    let client = auth_client();
    let body = urlencode(&[
        ("grant_type", DEVICE_GRANT_TYPE),
        ("device_code", device_code),
        ("client_id", client_id),
    ]);
    let response = client
        .post(url.clone())
        .header(CLIENT_VERSION_HEADER, env!("CARGO_PKG_VERSION"))
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(body)
        .send()
        .map_err(|e| {
            Box::new(MoocClientError::ConnectionError(
                Method::POST,
                url.clone(),
                e.without_url(),
            ))
        })?;

    let status = response.status();
    if status.is_success() {
        let token = parse_token(response, &url)?;
        return Ok(DeviceTokenPoll::Authorized(Box::new(token)));
    }

    // RFC 8628 §3.5: every non-success state is a 400 with an OAuth error code.
    let obsolete_client = status == StatusCode::UPGRADE_REQUIRED;
    let body = read_text(response, &url)?;
    match parse_oauth_error_code(&body).as_deref() {
        Some("authorization_pending") => Ok(DeviceTokenPoll::Pending),
        Some("slow_down") => Ok(DeviceTokenPoll::SlowDown),
        Some("expired_token") => Err(Box::new(MoocClientError::DeviceCodeExpired)),
        Some("access_denied") => Err(Box::new(MoocClientError::DeviceAccessDenied)),
        _ => Err(Box::new(MoocClientError::HttpError {
            url,
            status,
            error: body,
            obsolete_client,
            message_key: None,
        })),
    }
}

/// Redeems a refresh token for a fresh token pair (OAuth2 refresh grant).
///
/// The backend rotates the refresh token, returning a new one; if a response
/// omits it, the old refresh token is carried over so the stored credentials
/// remain refreshable.
pub fn refresh_token(
    root_url: &Url,
    client_id: &str,
    refresh_token: &str,
) -> MoocClientResult<api::Token> {
    let url = oauth_url(root_url, "token")?;
    let client = auth_client();
    let body = urlencode(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ]);
    let response = client
        .post(url.clone())
        .header(CLIENT_VERSION_HEADER, env!("CARGO_PKG_VERSION"))
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(body)
        .send()
        .map_err(|e| {
            Box::new(MoocClientError::ConnectionError(
                Method::POST,
                url.clone(),
                e.without_url(),
            ))
        })?;

    if response.status().is_success() {
        let mut token = parse_token(response, &url)?;
        if token.refresh_token().is_none() {
            token.set_refresh_token(Some(RefreshToken::new(refresh_token.to_string())));
        }
        return Ok(token);
    }

    // RFC 6749 §5.2: a 400 with an OAuth `error` code (`invalid_grant`) means the
    // refresh token is permanently dead. Anything else is transient and must
    // never lead the caller to delete credentials.
    let status = response.status();
    let obsolete_client = status == StatusCode::UPGRADE_REQUIRED;
    let body = read_text(response, &url)?;
    if status == StatusCode::BAD_REQUEST {
        if let Some(error) = parse_oauth_error_code(&body) {
            return Err(Box::new(MoocClientError::RefreshTokenRejected { error }));
        }
    }
    Err(Box::new(MoocClientError::HttpError {
        url,
        status,
        error: body,
        obsolete_client,
        message_key: None,
    }))
}

/// Builds a URL under the backend's main-frontend OAuth mount.
fn oauth_url(root_url: &Url, endpoint: &str) -> MoocClientResult<Url> {
    root_url
        .join("/api/v0/main-frontend/oauth/")
        .and_then(|u| u.join(endpoint))
        .map_err(|e| Box::new(MoocClientError::UrlParse(endpoint.to_string(), e)))
}

fn parse_token(response: reqwest::blocking::Response, url: &Url) -> MoocClientResult<api::Token> {
    let bytes = read_bytes(response, url)?;
    serde_json::from_slice(&bytes).map_err(|e| {
        Box::new(MoocClientError::DeserializingResponse {
            url: url.clone(),
            error: Box::new(e),
        })
    })
}

fn read_bytes(response: reqwest::blocking::Response, url: &Url) -> MoocClientResult<bytes::Bytes> {
    response.bytes().map_err(|e| {
        Box::new(MoocClientError::ReadingResponseBody {
            method: Method::POST,
            url: url.clone(),
            error: Box::new(e),
        })
    })
}

fn read_text(response: reqwest::blocking::Response, url: &Url) -> MoocClientResult<String> {
    response.text().map_err(|e| {
        Box::new(MoocClientError::ReadingResponseBody {
            method: Method::POST,
            url: url.clone(),
            error: Box::new(e),
        })
    })
}

/// Maps a non-success response to an [`MoocClientError::HttpError`], flagging
/// `426 Upgrade Required` as an obsolete client (as the resource API does).
fn http_error(response: reqwest::blocking::Response, url: Url) -> Box<MoocClientError> {
    let status = response.status();
    let obsolete_client = status == StatusCode::UPGRADE_REQUIRED;
    let error = response.text().unwrap_or_default();
    Box::new(MoocClientError::HttpError {
        url,
        status,
        error,
        obsolete_client,
        message_key: None,
    })
}

/// The single field of an RFC 6749 / RFC 8628 error response the poll loop
/// needs. Parsed leniently so a non-conforming body yields `None`.
#[derive(Deserialize)]
struct OAuthErrorBody {
    #[serde(default)]
    error: Option<String>,
}

fn parse_oauth_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<OAuthErrorBody>(body)
        .ok()
        .and_then(|b| b.error)
}

#[cfg(test)]
mod test {
    use super::*;
    use mockito::{Matcher, Server};

    #[test]
    fn device_authorization_parses_response() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/device_authorization")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("client_id=tmc-vscode".to_string()),
                Matcher::Regex("scope=exercise-services".to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "device_code": "dev-code",
                    "user_code": "WXYZ-1234",
                    "verification_uri": "https://courses.mooc.fi/oauth_device",
                    "verification_uri_complete": "https://courses.mooc.fi/oauth_device?user_code=WXYZ-1234",
                    "expires_in": 900,
                    "interval": 5
                })
                .to_string(),
            )
            .create();
        let root: Url = server.url().parse().unwrap();
        let res = device_authorization(&root, "tmc-vscode").unwrap();
        assert_eq!(res.device_code, "dev-code");
        assert_eq!(res.user_code, "WXYZ-1234");
        assert_eq!(res.expires_in, 900);
        assert_eq!(res.interval, 5);
        assert!(res.verification_uri_complete.is_some());
    }

    #[test]
    fn device_authorization_defaults_interval() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/device_authorization")
            .with_body(
                serde_json::json!({
                    "device_code": "dev-code",
                    "user_code": "WXYZ-1234",
                    "verification_uri": "https://courses.mooc.fi/oauth_device",
                    "expires_in": 900
                })
                .to_string(),
            )
            .create();
        let root: Url = server.url().parse().unwrap();
        let res = device_authorization(&root, "tmc-vscode").unwrap();
        assert_eq!(res.interval, DEFAULT_POLL_INTERVAL_SECS);
        assert!(res.verification_uri_complete.is_none());
    }

    #[test]
    fn poll_maps_pending_and_slow_down() {
        let mut server = Server::new();
        let pending = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"authorization_pending"}"#)
            .expect(1)
            .create();
        let root: Url = server.url().parse().unwrap();
        assert!(matches!(
            poll_device_token(&root, "cid", "dc").unwrap(),
            DeviceTokenPoll::Pending
        ));
        pending.assert();

        let slow = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"slow_down"}"#)
            .expect(1)
            .create();
        assert!(matches!(
            poll_device_token(&root, "cid", "dc").unwrap(),
            DeviceTokenPoll::SlowDown
        ));
        slow.assert();
    }

    #[test]
    fn poll_maps_terminal_errors() {
        let mut server = Server::new();
        let root: Url = server.url().parse().unwrap();

        let expired = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"expired_token"}"#)
            .expect(1)
            .create();
        assert!(matches!(
            *poll_device_token(&root, "cid", "dc").unwrap_err(),
            MoocClientError::DeviceCodeExpired
        ));
        expired.assert();

        let denied = server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"access_denied"}"#)
            .expect(1)
            .create();
        assert!(matches!(
            *poll_device_token(&root, "cid", "dc").unwrap_err(),
            MoocClientError::DeviceAccessDenied
        ));
        denied.assert();
    }

    #[test]
    fn poll_authorized_returns_token() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(
                    "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code".to_string(),
                ),
                Matcher::Regex("device_code=dc".to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "access_token": "at",
                    "refresh_token": "rt",
                    "token_type": "bearer",
                    "expires_in": 3600
                })
                .to_string(),
            )
            .create();
        let root: Url = server.url().parse().unwrap();
        match poll_device_token(&root, "cid", "dc").unwrap() {
            DeviceTokenPoll::Authorized(token) => {
                assert_eq!(token.access_token().secret(), "at");
                assert_eq!(token.refresh_token().unwrap().secret(), "rt");
            }
            other => panic!("expected Authorized, got {other:?}"),
        }
    }

    #[test]
    fn poll_426_maps_to_obsolete() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(426)
            .with_body("obsolete")
            .create();
        let root: Url = server.url().parse().unwrap();
        match *poll_device_token(&root, "cid", "dc").unwrap_err() {
            MoocClientError::HttpError {
                obsolete_client, ..
            } => assert!(obsolete_client),
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn refresh_returns_rotated_token() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("grant_type=refresh_token".to_string()),
                Matcher::Regex("refresh_token=old".to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "access_token": "new-at",
                    "refresh_token": "new-rt",
                    "token_type": "bearer",
                    "expires_in": 3600
                })
                .to_string(),
            )
            .create();
        let root: Url = server.url().parse().unwrap();
        let token = refresh_token(&root, "cid", "old").unwrap();
        assert_eq!(token.access_token().secret(), "new-at");
        assert_eq!(token.refresh_token().unwrap().secret(), "new-rt");
    }

    #[test]
    fn refresh_invalid_grant_is_permanent_rejection() {
        // A 400 with an OAuth `error` code must surface as `RefreshTokenRejected`
        // so the caller deletes credentials rather than retrying.
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant"}"#)
            .create();
        let root: Url = server.url().parse().unwrap();
        match *refresh_token(&root, "cid", "old").unwrap_err() {
            MoocClientError::RefreshTokenRejected { error } => assert_eq!(error, "invalid_grant"),
            other => panic!("expected RefreshTokenRejected, got {other:?}"),
        }
    }

    #[test]
    fn refresh_5xx_is_transient_http_error() {
        // A 5xx must stay a plain `HttpError` so the caller keeps the credentials.
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(503)
            .with_body("service unavailable")
            .create();
        let root: Url = server.url().parse().unwrap();
        match *refresh_token(&root, "cid", "old").unwrap_err() {
            MoocClientError::HttpError { status, .. } => assert_eq!(status.as_u16(), 503),
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn refresh_400_without_oauth_error_is_transient() {
        // An unparseable 400 body is transient, not a token rejection.
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_status(400)
            .with_body("<html>bad gateway</html>")
            .create();
        let root: Url = server.url().parse().unwrap();
        match *refresh_token(&root, "cid", "old").unwrap_err() {
            MoocClientError::HttpError { status, .. } => assert_eq!(status.as_u16(), 400),
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn refresh_transport_failure_is_connection_error() {
        // Transport failures must be `ConnectionError`, never `RefreshTokenRejected`,
        // so credentials are kept. A refused connection stands in for a timeout.
        let root: Url = "http://127.0.0.1:1/".parse().unwrap();
        match *refresh_token(&root, "cid", "old").unwrap_err() {
            MoocClientError::ConnectionError(..) => {}
            other => panic!("expected ConnectionError, got {other:?}"),
        }
    }

    #[test]
    fn refresh_carries_over_old_refresh_token_when_omitted() {
        let mut server = Server::new();
        server
            .mock("POST", "/api/v0/main-frontend/oauth/token")
            .with_body(
                serde_json::json!({
                    "access_token": "new-at",
                    "token_type": "bearer",
                    "expires_in": 3600
                })
                .to_string(),
            )
            .create();
        let root: Url = server.url().parse().unwrap();
        let token = refresh_token(&root, "cid", "old-rt").unwrap();
        assert_eq!(token.refresh_token().unwrap().secret(), "old-rt");
    }
}
