//! What the host accepts as an authored candidate document.

/// The bytes the host will treat as the candidate document.
///
/// A model that returns the document alone is passed through untouched. One
/// that wraps it in a single Markdown fence has still authored exactly one
/// artifact, and the host — not the model — decides that the fence is packaging
/// rather than content, so it is unwrapped. Anything else, including several
/// fenced blocks, is left exactly as it arrived: the parse then fails and the
/// author is told what to fix, which is safer than guessing which block was
/// meant. Nothing here validates the document; that stays with the gate.
pub(crate) fn candidate_document_bytes(candidate: &[u8]) -> &[u8] {
    let Ok(text) = std::str::from_utf8(candidate) else {
        return candidate;
    };
    if text.trim_start().starts_with(['{', '[']) {
        return first_json_document(candidate);
    }
    let mut blocks = text.split("```");
    let _before = blocks.next();
    let mut only: Option<&str> = None;
    while let Some(block) = blocks.next() {
        if blocks.next().is_none() && block.trim().is_empty() {
            break;
        }
        let body = block.strip_prefix("json").unwrap_or(block);
        if only.is_some() {
            return candidate;
        }
        only = Some(body);
    }
    match only {
        Some(body) if body.trim_start().starts_with(['{', '[']) => {
            first_json_document(body.as_bytes())
        }
        _ => candidate,
    }
}

/// The first complete JSON value in `bytes`, or all of `bytes` if there is none.
///
/// A reply that opens with the artifact and then explains itself has still
/// authored exactly one document; the commentary after it is packaging the host
/// discards, the same way it discards a fence. Truncating at the parser's own
/// end offset keeps that decision exact rather than heuristic — nothing is
/// searched for, and a reply whose first value never closes is returned whole so
/// the parse fails and the author is told.
fn first_json_document(bytes: &[u8]) -> &[u8] {
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<serde_json::Value>();
    match stream.next() {
        Some(Ok(_)) => &bytes[..stream.byte_offset()],
        _ => bytes,
    }
}

/// Why `candidate` is not the document the command stages, if it is not.
///
/// The two failures need different words: a reply that is not JSON at all is a
/// packaging problem, while one that parses but lacks a field is a shape
/// problem, and telling an author to remove code fences it did not emit sends
/// it to repair the wrong thing.
pub(crate) fn candidate_parse_error<T: serde::de::DeserializeOwned>(
    candidate: &[u8],
) -> Option<String> {
    let document = candidate_document_bytes(candidate);
    let Err(error) = serde_json::from_slice::<T>(document) else {
        return None;
    };
    if serde_json::from_slice::<serde_json::Value>(document).is_err() {
        return Some(format!("the reply is not a JSON document ({error})"));
    }
    Some(format!(
        "the JSON document does not match the required shape ({error})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_fenced_document_is_unwrapped_by_the_host() {
        let raw = b"Here it is.\n\n```json\n{\"kind\":\"x\"}\n```\n";
        assert_eq!(
            candidate_document_bytes(raw).trim_ascii(),
            b"{\"kind\":\"x\"}"
        );
        assert!(candidate_parse_error::<serde_json::Value>(raw).is_none());
    }

    #[test]
    fn a_document_followed_by_commentary_is_truncated_to_the_document() {
        let raw = b"{\"kind\":\"x\"}\n\nNotes: this satisfies AC-1.\n";
        assert_eq!(candidate_document_bytes(raw), b"{\"kind\":\"x\"}");
        assert!(candidate_parse_error::<serde_json::Value>(raw).is_none());
    }

    #[test]
    fn a_document_that_never_closes_is_returned_whole_and_refused() {
        let raw = b"{\"kind\": \"x\"";
        assert_eq!(candidate_document_bytes(raw), raw);
        assert!(candidate_parse_error::<serde_json::Value>(raw).is_some());
    }

    #[test]
    fn a_bare_document_is_passed_through_untouched() {
        let raw = b"{\"kind\":\"x\"}";
        assert_eq!(candidate_document_bytes(raw), raw);
    }

    #[test]
    fn several_fenced_blocks_are_refused_rather_than_guessed() {
        let raw = b"one\n```json\n{\"a\":1}\n```\ntwo\n```json\n{\"b\":2}\n```\n";
        assert_eq!(candidate_document_bytes(raw), raw);
        assert!(candidate_parse_error::<serde_json::Value>(raw).is_some());
    }

    #[test]
    fn prose_with_no_document_is_refused_as_packaging_not_shape() {
        let reason =
            candidate_parse_error::<serde_json::Value>(b"All context gathered. No artifact.\n")
                .expect("prose carrying no document does not deserialize");

        assert!(reason.contains("not a JSON document"), "{reason}");
    }

    #[test]
    fn a_parsing_document_of_the_wrong_shape_is_reported_as_shape() {
        #[derive(serde::Deserialize)]
        struct Wanted {
            #[allow(dead_code)]
            schema_version: u32,
        }

        let reason = candidate_parse_error::<Wanted>(b"{\"other\":1}")
            .expect("a document missing a required field does not deserialize");

        assert!(
            reason.contains("does not match the required shape"),
            "{reason}"
        );
    }
}
