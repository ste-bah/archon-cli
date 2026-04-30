//! TASK-AGS-707: SSE + NDJSON line decoders for `OpenAiCompatProvider::stream`.
//!
//! Used by the streaming path to translate raw wire-format frames into
//! `StreamEvent`s without per-provider branching. The delimiter enum
//! (`StreamDelimiter::Sse` vs `MistralNdjson`) from TASK-AGS-705 quirks
//! decides which decoder the caller invokes — per-provider code paths are
//! strictly forbidden in the stream body itself.
//!
//! Tolerant of unknown fields per EC-PROV-01: `parse_openai_sse_chunk`
//! (re-used from `providers::openai`) ignores fields it doesn't recognize
//! rather than erroring, so new OpenAI-compat providers can add wire
//! extensions without breaking the decoder.

use crate::streaming::StreamEvent;

use super::openai::parse_openai_sse_chunk;

/// Outcome of decoding a single line from a streaming wire.
#[derive(Debug)]
pub(crate) enum FrameOutcome {
    /// Zero or more `StreamEvent`s produced from a data frame. An empty
    /// vec is legal (e.g. an OpenAI chunk that carries only metadata with
    /// no content delta) — callers should forward each event verbatim.
    Events(Vec<StreamEvent>),
    /// End-of-stream sentinel. SSE: `data: [DONE]`. NDJSON has no
    /// sentinel — callers signal end-of-stream on network EOF instead.
    End,
}

/// Decode one SSE line.
///
/// Returns `None` for:
///   - empty lines (SSE event separators)
///   - `:`-prefixed comment / keepalive lines (SSE spec says ignore)
///   - lines that aren't `data:` (e.g. `event:`, `id:`, `retry:`) —
///     OpenAI-compat streams don't use those fields and our caller has no
///     use for them.
///
/// A `data: {json}` line returns `Some(Events(vec))`. The `[DONE]` sentinel
/// returns `Some(End)`.
pub(crate) fn decode_sse_line(line: &[u8]) -> Option<FrameOutcome> {
    let s = std::str::from_utf8(line).ok()?;
    // Strip any trailing CR (SSE lines may end `\r\n`).
    let s = s.trim_end_matches('\r');
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    // SSE comment: any line starting with `:` is a comment / keepalive.
    if trimmed.starts_with(':') {
        return None;
    }
    // Only `data:` fields carry payload. `event:`, `id:`, `retry:` are
    // valid SSE fields but are no-ops for OpenAI-compat streams.
    let rest = trimmed.strip_prefix("data:")?;
    let payload = rest.trim();
    if payload == "[DONE]" {
        return Some(FrameOutcome::End);
    }
    Some(FrameOutcome::Events(parse_openai_sse_chunk(payload)))
}

/// Decode one NDJSON line (Mistral-style streaming).
///
/// Mistral emits `{json}\n` — no `data:` prefix, no `[DONE]` sentinel
/// (end-of-stream is signaled by the server closing the connection).
/// Empty/whitespace-only lines return `None`.
pub(crate) fn decode_ndjson_line(line: &[u8]) -> Option<FrameOutcome> {
    let s = std::str::from_utf8(line).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(FrameOutcome::Events(parse_openai_sse_chunk(trimmed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_sse_skips_comments() {
        // Validation Criterion 4: `: keepalive` must return None.
        assert!(decode_sse_line(b": keepalive").is_none());
        assert!(decode_sse_line(b":ping").is_none());
        assert!(decode_sse_line(b"").is_none());
        assert!(decode_sse_line(b"   ").is_none());
    }

    #[test]
    fn decode_sse_parses_done_sentinel() {
        // Validation Criterion 5: `[DONE]` produces stream end, not a chunk.
        match decode_sse_line(b"data: [DONE]") {
            Some(FrameOutcome::End) => {}
            other => panic!("expected End, got {other:?}"),
        }
        // Also tolerate no-space form.
        match decode_sse_line(b"data:[DONE]") {
            Some(FrameOutcome::End) => {}
            other => panic!("expected End, got {other:?}"),
        }
    }

    #[test]
    fn decode_sse_parses_text_delta_chunk() {
        let line = br#"data: {"choices":[{"delta":{"content":"hi"}}]}"#;
        match decode_sse_line(line) {
            Some(FrameOutcome::Events(events)) => {
                assert!(!events.is_empty(), "expected at least one event");
                let has_text = events
                    .iter()
                    .any(|e| matches!(e, StreamEvent::TextDelta { text, .. } if text == "hi"));
                assert!(
                    has_text,
                    "expected TextDelta{{text:\"hi\"}}, got {events:?}"
                );
            }
            other => panic!("expected Events, got {other:?}"),
        }
    }

    #[test]
    fn decode_sse_tolerates_unknown_fields() {
        // EC-PROV-01: unknown fields must not cause errors.
        let line =
            br#"data: {"choices":[{"delta":{"content":"hi","unknown":42}}],"xyz":"ignored"}"#;
        assert!(matches!(
            decode_sse_line(line),
            Some(FrameOutcome::Events(_))
        ));
    }

    #[test]
    fn decode_sse_ignores_non_data_fields() {
        assert!(decode_sse_line(b"event: ping").is_none());
        assert!(decode_sse_line(b"id: 42").is_none());
        assert!(decode_sse_line(b"retry: 1000").is_none());
    }

    #[test]
    fn decode_sse_handles_crlf() {
        // `trim_end_matches('\r')` strips the CR. The payload is then
        // parsed normally.
        let line = b"data: [DONE]\r";
        match decode_sse_line(line) {
            Some(FrameOutcome::End) => {}
            other => panic!("expected End for CR-terminated [DONE], got {other:?}"),
        }
    }

    #[test]
    fn decode_ndjson_skips_empty() {
        assert!(decode_ndjson_line(b"").is_none());
        assert!(decode_ndjson_line(b"   ").is_none());
        assert!(decode_ndjson_line(b"\t").is_none());
    }

    #[test]
    fn decode_ndjson_parses_line() {
        let line = br#"{"choices":[{"delta":{"content":"bonjour"}}]}"#;
        match decode_ndjson_line(line) {
            Some(FrameOutcome::Events(events)) => {
                assert!(!events.is_empty());
                let has_text = events
                    .iter()
                    .any(|e| matches!(e, StreamEvent::TextDelta { text, .. } if text == "bonjour"));
                assert!(has_text, "expected TextDelta{{text:\"bonjour\"}}");
            }
            other => panic!("expected Events, got {other:?}"),
        }
    }

    #[test]
    fn decode_ndjson_tolerates_unknown_fields() {
        let line = br#"{"choices":[{"delta":{"content":"x"}}],"_meta":{"provider":"mistral"}}"#;
        assert!(matches!(
            decode_ndjson_line(line),
            Some(FrameOutcome::Events(_))
        ));
    }
}
