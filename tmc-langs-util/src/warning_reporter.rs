//! Contains an utility for reporting warnings.

use once_cell::sync::OnceCell;
use serde::Serialize;

type WarningClosure = Box<dyn 'static + Sync + Send + Fn(Warning)>;

static WARNING_REPORTER: OnceCell<WarningClosure> = OnceCell::new();

/// Initializes the warning reporter with the given closure to be called with any warnings.
/// Can only be initialized once, repeated calls do nothing.
pub fn init(reporter: WarningClosure) {
    WARNING_REPORTER.get_or_init(|| reporter);
}

/// Calls the warning closure with the given warning.
pub fn warn(warning: Warning) {
    if let Some(reporter) = WARNING_REPORTER.get() {
        reporter(warning);
    }
}

#[derive(Debug, Serialize)]
pub struct Warning {
    warning: String,
}

impl Warning {
    pub fn new(message: impl ToString) -> Self {
        Self {
            warning: message.to_string(),
        }
    }
}
