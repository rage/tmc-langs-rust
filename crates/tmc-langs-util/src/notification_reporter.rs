//! Contains an utility for reporting warnings.

use once_cell::sync::OnceCell;
use serde::Serialize;

type NotificationClosure = Box<dyn 'static + Sync + Send + Fn(Notification)>;

static NOTIFICATION_REPORTER: OnceCell<NotificationClosure> = OnceCell::new();

/// Initializes the warning reporter with the given closure to be called with any warnings.
/// Can only be initialized once, repeated calls do nothing.
pub fn init(reporter: NotificationClosure) {
    NOTIFICATION_REPORTER.get_or_init(|| reporter);
}

/// Calls the warning closure with the given warning.
pub fn notify(notification: Notification) {
    if let Some(reporter) = NOTIFICATION_REPORTER.get() {
        reporter(notification);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Notification {
    notification_kind: NotificationKind,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Warning,
    Info,
}

impl Notification {
    pub fn warning(message: impl ToString) -> Self {
        Self {
            notification_kind: NotificationKind::Warning,
            message: message.to_string(),
        }
    }

    pub fn info(message: impl ToString) -> Self {
        Self {
            notification_kind: NotificationKind::Info,
            message: message.to_string(),
        }
    }
}
