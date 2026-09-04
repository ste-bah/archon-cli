//! The host's tolerance for an authored JSON document, offered to every path
//! that accepts one: the same two unambiguous repairs the agent envelope gets
//! (a trailing comma before a closer, a closer written where a different one
//! was owed), and the same fault excerpt when neither applies. A document that
//! stops mid-value is never completed; it is described so the author can fix it.
use super::agent_output_fault::EnvelopeParseError;
use super::agent_output_normalize::strip_trailing_commas;
use super::agent_output_tolerance::repair_mismatched_closers;

/// A repaired copy of `text` that parses as JSON, when `text` itself does not
/// and one of the two repairs (a trailing comma, a closer written early)
/// produces a document. Only closers are ever inserted, never removed, so a
/// spurious opener is not repaired here; the typed parse then names the shape.
/// `None` when `text` already parses, when it is truncated, or when neither
/// repair applies.
pub fn repair_json_document(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return None;
    }
    let stripped = strip_trailing_commas(trimmed);
    if stripped.len() != trimmed.len()
        && serde_json::from_str::<serde_json::Value>(&stripped).is_ok()
    {
        return Some(stripped);
    }
    let completed = repair_mismatched_closers(&stripped)?;
    serde_json::from_str::<serde_json::Value>(&completed)
        .ok()
        .map(|_| completed)
}

/// Why `text` is not a JSON document, with the bytes at fault marked and the
/// hint that names the fix. `None` when `text` parses.
pub fn describe_json_fault(text: &str) -> Option<String> {
    let trim_start = text.len() - text.trim_start().len();
    match serde_json::from_str::<serde_json::Value>(text.trim()) {
        Ok(_) => None,
        Err(error) => Some(EnvelopeParseError::new(error, trim_start).describe(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_comma_and_an_early_closer_each_have_one_reading() {
        assert_eq!(
            repair_json_document(r#"{"a":[1,2,],}"#).as_deref(),
            Some(r#"{"a":[1,2]}"#)
        );
        assert_eq!(
            repair_json_document(r#"{"a":[{"b":1]}"#).as_deref(),
            Some(r#"{"a":[{"b":1}]}"#)
        );
        assert!(
            repair_json_document(r#"{"a":1}"#).is_none(),
            "valid: nothing to do"
        );
    }

    #[test]
    fn a_truncated_document_is_described_not_completed() {
        let cut = r#"{"a":[{"b":"unterminated"#;
        assert!(repair_json_document(cut).is_none());
        let described = describe_json_fault(cut).expect("not a document");
        assert!(described.contains("reply ends"), "{described}");
        assert!(describe_json_fault(r#"{"a":1}"#).is_none());
    }

    #[test]
    fn an_interior_fault_is_marked_where_it_sits() {
        let bad = r#"{"a":1 "b":2}"#;
        let described = describe_json_fault(bad).expect("not a document");
        assert!(described.contains("<HERE>"), "{described}");
        assert!(described.contains("expected `,` or `}`"), "{described}");
    }
}
