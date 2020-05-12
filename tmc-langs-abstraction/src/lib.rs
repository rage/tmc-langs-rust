//! Contains ValidationResult and related structs and enums.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub enum Strategy {
    Fail,
    Warn,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ValidationError {
    column: usize,
    line: usize,
    message: String,
    source: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ValidationResult {
    strategy: Strategy,
    validation_errors: HashMap<PathBuf, Vec<ValidationError>>,
}
