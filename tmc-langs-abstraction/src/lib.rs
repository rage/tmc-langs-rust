//! Contains ValidationResult and related structs and enums.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Strategy {
    Fail,
    Warn,
    Disabled,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    pub column: usize,
    pub line: usize,
    pub message: String,
    pub source_name: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub strategy: Strategy,
    pub validation_errors: Option<HashMap<PathBuf, Vec<ValidationError>>>,
}
