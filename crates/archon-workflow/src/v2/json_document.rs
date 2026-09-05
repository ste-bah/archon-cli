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
    if let Some(completed) = repair_mismatched_closers(&stripped)
        && serde_json::from_str::<serde_json::Value>(&completed).is_ok()
    {
        return Some(completed);
    }
    repair_local_slips(&stripped)
}

/// The local slips an author makes in a large document, each fixed where the
/// parser trips, one at a time, until the document parses or no rule applies:
/// an element that opens where a key was expected (the previous element was
/// never closed: close it before the comma); an array closer while an object
/// is still open (close the object first); closers left over after the root
/// value (drop them); and a bare `"` that ends a string early with content,
/// not structure, after it (it was meant as content: escape it). Bounded, and
/// validated by the parse that has to succeed at the end.
pub fn repair_local_slips(text: &str) -> Option<String> {
    let mut doc = text.to_string();
    for _ in 0..128 {
        let error = match serde_json::from_str::<serde_json::Value>(&doc) {
            Ok(_) => return Some(doc),
            Err(error) => error,
        };
        let message = error.to_string();
        let offset = byte_offset(&doc, error.line(), error.column())?;
        let at = doc[offset..].chars().next();
        if message.starts_with("key must be a string") && at == Some('{') {
            let comma = doc[..offset].rfind(',')?;
            doc.insert(comma, '}');
            continue;
        }
        if message.starts_with("expected `,` or `}`") && at == Some(']') {
            // The object is closed before the array is; a key the author meant
            // for the object may land one level up, where the typed parse
            // decides whether it belongs. The alternative reading (the array
            // closer was the slip) has no signal here.
            doc.insert(offset, '}');
            continue;
        }
        if message.starts_with("trailing characters") {
            let rest = doc[offset..].trim();
            if !rest.is_empty() && rest.chars().all(|c| c == '}' || c == ']') {
                doc.truncate(offset);
                let end = doc.trim_end().len();
                doc.truncate(end);
                continue;
            }
            return None;
        }
        if message.starts_with("expected `,` or")
            || message.starts_with("key must be a string")
            || message.starts_with("expected value")
        {
            let quote = doc[..offset].rfind('"')?;
            if quote > 0 && doc.as_bytes()[quote - 1] == b'\\' {
                return None;
            }
            let after = doc[quote + 1..].trim_start();
            if after.is_empty() || after.starts_with([',', '}', ']', ':']) {
                return None;
            }
            // Two strings with only whitespace between them are a missing
            // comma, not a bare quote: escaping would merge them into one.
            if after.starts_with('"') {
                return None;
            }
            doc.insert(quote, '\\');
            continue;
        }
        return None;
    }
    None
}

fn byte_offset(text: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let line_start = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    let offset = line_start + column.saturating_sub(1);
    (offset <= text.len() && text.is_char_boundary(offset)).then_some(offset)
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

    /// The live slips, in miniature: a bare quote inside an embedded command;
    /// entries that never close before the next one opens, with the closers
    /// piled up at the end; and an array closed while an object is open.
    #[test]
    fn local_slips_in_a_large_document_have_one_reading_each() {
        let bare = r#"{"check":{"command":"grep -qE '([^a-z])'"$p"'([^a-z]|$)' out"},"ok":true}"#;
        let fixed = repair_json_document(bare).expect("bare quotes re-escaped");
        let value: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(
            value["check"]["command"],
            "grep -qE '([^a-z])'\"$p\"'([^a-z]|$)' out"
        );

        let unclosed = r#"{"items":[{"id":"a","judgment":{"v":1},{"id":"b","judgment":{"v":2},{"id":"c","judgment":{"v":3}}]}}"#;
        let fixed = repair_json_document(unclosed).expect("each entry closed before the next");
        let value: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(value["items"].as_array().map(Vec::len), Some(3));
        assert_eq!(value["items"][2]["judgment"]["v"], 3);

        let array_closed_early = r#"{"items":[{"id":"a","judgment":{"v":1}],"extra":[]}"#;
        let fixed =
            repair_json_document(array_closed_early).expect("object closed before the array");
        let value: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(value["items"][0]["id"], "a");

        assert!(
            repair_json_document(r#"{"a":"unterminated"#).is_none(),
            "truncation stays refused"
        );
        assert!(
            repair_json_document(r#"["a" "b"]"#).is_none(),
            "two strings with a missing comma are not merged into one"
        );
    }

    #[test]
    fn an_interior_fault_is_marked_where_it_sits() {
        let bad = r#"{"a":1 "b":2}"#;
        let described = describe_json_fault(bad).expect("not a document");
        assert!(described.contains("<HERE>"), "{described}");
        assert!(described.contains("expected `,` or `}`"), "{described}");
    }
}

#[cfg(test)]
mod diagnostic {
    /// Diagnostic, not a test of record: point `ARCHON_JSON_DOCUMENT` at a
    /// refused reply and see whether the host's repairs read it.
    #[test]
    #[ignore = "diagnostic; requires ARCHON_JSON_DOCUMENT"]
    fn repairs_the_document_at_env_path() {
        let path = std::env::var("ARCHON_JSON_DOCUMENT").expect("ARCHON_JSON_DOCUMENT");
        let text = std::fs::read_to_string(&path).expect("readable");
        match super::repair_json_document(&text) {
            Some(fixed) => println!("REPAIRED {} bytes -> {} bytes", text.len(), fixed.len()),
            None => println!(
                "NOT REPAIRED: {}",
                super::describe_json_fault(&text).unwrap_or_else(|| "parses as is".into())
            ),
        }
    }
}
