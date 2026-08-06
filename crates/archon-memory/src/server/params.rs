//! JSON-RPC parameter extraction for [`super::dispatch`].
//!
//! Split out of `server.rs` only to keep that file under the project's
//! 500-line gate; nothing here knows about the transport.

use serde_json::Value;

use crate::board::BoardStatus;
use crate::types::{MemoryType, RelType};

pub(super) fn str_param(params: &Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("missing or invalid string param: {key}"))
}

pub(super) fn opt_str_param(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(String::from)
}

pub(super) fn f64_param(params: &Value, key: &str) -> Result<f64, String> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing or invalid f64 param: {key}"))
}

pub(super) fn usize_param(params: &Value, key: &str) -> Result<usize, String> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .ok_or_else(|| format!("missing or invalid usize param: {key}"))
}

pub(super) fn string_array_param(params: &Value, key: &str) -> Result<Vec<String>, String> {
    let arr = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or invalid array param: {key}"))?;
    arr.iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| format!("non-string element in {key}"))
        })
        .collect()
}

pub(super) fn opt_string_array_param(params: &Value, key: &str) -> Option<Vec<String>> {
    params
        .get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_array() })
        .and_then(|arr| {
            arr.iter()
                .map(|v| v.as_str().map(String::from))
                .collect::<Option<Vec<_>>>()
        })
}

pub(super) fn memory_type_param(params: &Value, key: &str) -> Result<MemoryType, String> {
    let s = str_param(params, key)?;
    // Support both enum variant names ("Fact") and stored format ("fact")
    MemoryType::from_str_opt(&s)
        .or_else(|| MemoryType::from_str_opt(&s.to_lowercase()))
        .ok_or_else(|| format!("invalid memory type: {s}"))
}

/// Parse a board status sent as its stored form (`in_review`), tolerating the
/// enum variant name (`InReview`) the way the memory-type parser does.
pub(super) fn board_status_param(params: &Value, key: &str) -> Result<BoardStatus, String> {
    let raw = str_param(params, key)?;
    parse_board_status(&raw)
}

fn parse_board_status(raw: &str) -> Result<BoardStatus, String> {
    BoardStatus::from_str_opt(raw)
        .or_else(|| BoardStatus::from_str_opt(&pascal_to_snake(raw)))
        .ok_or_else(|| format!("invalid board status: {raw}"))
}

/// An absent or null `statuses` means "every status", which is what an empty
/// slice means to the query -- a caller polling the whole board should not have
/// to enumerate the lifecycle.
pub(super) fn board_status_array_param(
    params: &Value,
    key: &str,
) -> Result<Vec<BoardStatus>, String> {
    let Some(array) = params.get(key).and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("non-string element in {key}"))
                .and_then(parse_board_status)
        })
        .collect()
}

pub(super) fn rel_type_param(params: &Value, key: &str) -> Result<RelType, String> {
    let s = str_param(params, key)?;
    // Support both enum variant names ("RelatedTo") and stored format ("related_to")
    RelType::from_str_opt(&s)
        .or_else(|| {
            // Convert PascalCase to snake_case for lookup
            let snake = pascal_to_snake(&s);
            RelType::from_str_opt(&snake)
        })
        .ok_or_else(|| format!("invalid relationship type: {s}"))
}

/// Simple PascalCase to snake_case converter for enum variant matching.
fn pascal_to_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}
