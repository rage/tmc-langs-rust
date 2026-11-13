//! User-facing wording of a backend's error response, shared by both backend clients.

use std::fmt::Display;

/// Longest response body quoted in an error message, in characters.
const MAX_QUOTED_BODY_CHARS: usize = 300;

/// The sentence a failed request reads as: `"{status} from {url}"`, then `": {detail}"` when
/// the backend said something, then a note when the client is too old for the backend.
///
/// `url` must already be safe to show (see the mooc client's query redaction).
pub fn describe_http_error(
    status: impl Display,
    url: &str,
    detail: Option<&str>,
    obsolete_client: bool,
) -> String {
    let mut message = format!("{status} from {url}");
    if let Some(detail) = detail {
        message.push_str(": ");
        message.push_str(detail);
    }
    if obsolete_client {
        if !message.ends_with(['.', '!', '?']) {
            message.push('.');
        }
        message.push_str(" This client version is no longer supported; update it.");
    }
    message
}

/// A response body fit to quote in an error message: trimmed and shortened, or `None` when it
/// is empty or an HTML page (a proxy's or web server's error page says nothing useful).
pub fn quotable_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() || body.starts_with('<') {
        return None;
    }
    let mut chars = body.chars();
    let mut quoted: String = chars.by_ref().take(MAX_QUOTED_BODY_CHARS).collect();
    if chars.next().is_some() {
        quoted.push('…');
    }
    Some(quoted)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn leaves_out_an_absent_detail() {
        assert_eq!(
            describe_http_error("404 Not Found", "https://host/x", None, false),
            "404 Not Found from https://host/x"
        );
    }

    #[test]
    fn appends_the_detail() {
        assert_eq!(
            describe_http_error(
                "422 Unprocessable Entity",
                "https://host/x",
                Some("Exercise deadline passed."),
                false
            ),
            "422 Unprocessable Entity from https://host/x: Exercise deadline passed."
        );
    }

    #[test]
    fn mentions_an_obsolete_client_only_when_it_is_one() {
        assert_eq!(
            describe_http_error(
                "426 Upgrade Required",
                "https://host/x",
                Some("Too old."),
                true
            ),
            "426 Upgrade Required from https://host/x: Too old. \
             This client version is no longer supported; update it."
        );
        assert!(!describe_http_error("400 Bad Request", "u", None, false).contains("client"));
    }

    #[test]
    fn quotable_body_drops_empty_and_html_bodies() {
        assert_eq!(quotable_body("  \n"), None);
        assert_eq!(quotable_body("<html><body>404</body></html>"), None);
        assert_eq!(
            quotable_body(" plain text \n"),
            Some("plain text".to_string())
        );
    }

    #[test]
    fn quotable_body_shortens_a_long_body() {
        let quoted = quotable_body(&"x".repeat(1000)).unwrap();
        assert_eq!(quoted.chars().count(), MAX_QUOTED_BODY_CHARS + 1);
        assert!(quoted.ends_with('…'));
    }
}
