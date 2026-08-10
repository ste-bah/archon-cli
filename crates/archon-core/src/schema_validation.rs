//! JSON Schema validation for `--json-schema` output mode (CLI-227).
//!
//! Provides helpers to extract JSON from assistant text (which may be wrapped
//! in markdown code blocks) and validate it against a user-supplied JSON schema.
//!
//! The fence-stripping itself lives in [`archon_context::fenced`] — five copies
//! of it had accumulated across the workspace, and this one could not host the
//! shared version because `archon-core` depends on `archon-memory`, which is
//! one of the callers.

use archon_context::fenced::{fenced_block_tagged, first_fenced_block};

/// Validate a JSON string against a JSON schema.
///
/// Returns `Ok(())` when the JSON conforms to the schema, or `Err` with a list
/// of human-readable validation error messages.
pub fn validate_json_schema(json_str: &str, schema_str: &str) -> Result<(), Vec<String>> {
    let schema_value: serde_json::Value = serde_json::from_str(schema_str)
        .map_err(|e| vec![format!("Failed to parse JSON schema: {e}")])?;

    let instance: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| vec![format!("Failed to parse JSON input: {e}")])?;

    let validator = jsonschema::validator_for(&schema_value)
        .map_err(|e| vec![format!("Invalid JSON schema: {e}")])?;

    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{e}"))
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Extract JSON from assistant text.
///
/// Tries the following strategies in order:
/// 1. Parse the entire text as JSON directly.
/// 2. Extract from a `` ```json ... ``` `` fenced code block.
/// 3. Extract from a `` ``` ... ``` `` fenced code block (no language tag).
///
/// Returns the extracted JSON string, or `None` if no valid JSON is found.
pub fn extract_json(text: &str) -> Option<String> {
    let trimmed = text.trim();

    // Strategy 1: Try parsing the whole text as JSON
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }

    // Strategies 2 and 3: a ```json block, then any fenced block. The tagged
    // one is tried first because a response can carry several blocks and the
    // tagged one is the answer; the untagged pass then covers a bare fence.
    for block in [
        fenced_block_tagged(trimmed, "json"),
        first_fenced_block(trimmed),
    ]
    .into_iter()
    .flatten()
    {
        let body = block.body.trim();
        if !body.is_empty() && serde_json::from_str::<serde_json::Value>(body).is_ok() {
            return Some(body.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_from_triple_backtick_json() {
        let input = "Here is the result:\n\n```json\n{\"key\": \"value\"}\n```\n\nDone.";
        let result = extract_json(input);
        assert!(result.is_some());
        let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn extract_from_plain_backtick() {
        let input = "Output:\n\n```\n{\"x\": 42}\n```";
        let result = extract_json(input);
        assert!(result.is_some());
    }
}
