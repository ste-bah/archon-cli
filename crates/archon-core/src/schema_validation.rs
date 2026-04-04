//! JSON Schema validation for `--json-schema` output mode (CLI-227).
//!
//! Provides helpers to extract JSON from assistant text (which may be wrapped
//! in markdown code blocks) and validate it against a user-supplied JSON schema.

/// Validate a JSON string against a JSON schema.
///
/// Returns `Ok(())` when the JSON conforms to the schema, or `Err` with a list
/// of human-readable validation error messages.
pub fn validate_json_schema(json_str: &str, schema_str: &str) -> Result<(), Vec<String>> {
    let schema_value: serde_json::Value = serde_json::from_str(schema_str).map_err(|e| {
        vec![format!("Failed to parse JSON schema: {e}")]
    })?;

    let instance: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
        vec![format!("Failed to parse JSON input: {e}")]
    })?;

    let validator = jsonschema::validator_for(&schema_value).map_err(|e| {
        vec![format!("Invalid JSON schema: {e}")]
    })?;

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

    // Strategy 2: Extract from ```json ... ``` block
    if let Some(extracted) = extract_from_fenced_block(trimmed, "```json") {
        if serde_json::from_str::<serde_json::Value>(&extracted).is_ok() {
            return Some(extracted);
        }
    }

    // Strategy 3: Extract from ``` ... ``` block (no language tag)
    if let Some(extracted) = extract_from_fenced_block(trimmed, "```") {
        if serde_json::from_str::<serde_json::Value>(&extracted).is_ok() {
            return Some(extracted);
        }
    }

    None
}

/// Extract content between the first occurrence of `open_fence` and the next
/// closing `` ``` ``.
fn extract_from_fenced_block(text: &str, open_fence: &str) -> Option<String> {
    let start_idx = text.find(open_fence)?;
    let after_fence = &text[start_idx + open_fence.len()..];

    // Skip to the next newline (the fence line itself may have trailing text)
    let content_start = after_fence.find('\n').map(|i| i + 1)?;
    let content = &after_fence[content_start..];

    // Find closing fence
    let end_idx = content.find("```")?;
    let extracted = content[..end_idx].trim().to_string();

    if extracted.is_empty() {
        None
    } else {
        Some(extracted)
    }
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
