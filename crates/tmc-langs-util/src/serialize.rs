//! Utility functions for de/serializing data wrapped with serde_path_to_error for better errors.

use crate::JsonError;
use serde::Serialize;

pub fn to_json_string<T: Serialize>(value: &T) -> Result<String, JsonError> {
    let mut buf = Vec::new();
    let se = &mut serde_json::Serializer::new(&mut buf);
    serde_path_to_error::serialize(value, se)?;
    let string = String::from_utf8(buf).expect("invalid json from serializer");
    Ok(string)
}

pub fn to_json_vec<T: Serialize>(value: &T) -> Result<Vec<u8>, JsonError> {
    let mut buf = Vec::new();
    let se = &mut serde_json::Serializer::new(&mut buf);
    serde_path_to_error::serialize(value, se)?;
    Ok(buf)
}

pub fn to_json_value<T: Serialize>(value: &T) -> Result<serde_json::Value, JsonError> {
    let se = serde_json::value::Serializer;
    let value = serde_path_to_error::serialize(value, se)?;
    Ok(value)
}
