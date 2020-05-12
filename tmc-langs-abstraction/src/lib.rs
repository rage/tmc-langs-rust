//! Contains ValidationResult and related structs and enums.

use std::collections::HashMap;
use std::path::PathBuf;

pub enum Strategy {
    Fail,
    Warn,
}

pub struct ValidationError {
    column: usize,
    line: usize,
    message: String,
    source: String,
}

pub struct ValidationResult {
    strategy: Strategy,
    validation_errors: HashMap<PathBuf, Vec<ValidationError>>,
}
