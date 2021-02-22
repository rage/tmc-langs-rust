use once_cell::sync::OnceCell;
use serde::Serialize;

type WarningClosure = Box<dyn 'static + Sync + Send + Fn(Warning)>;

pub static WARNING_REPORTER: OnceCell<WarningClosure> = OnceCell::new();

pub fn init(reporter: WarningClosure) {
    WARNING_REPORTER.get_or_init(|| reporter);
}

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
