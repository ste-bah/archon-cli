//! Safe debug-log body formatting — port of Claude Code's `debugUtils.ts`.
//!
//! Source of truth: `project-zero/bridge/debugUtils.ts` lines 9-52.
//!
//! # Behavioural deviation from the TypeScript source
//!
//! `debug_body()` delegates to `debug_truncate()`, which collapses `\n` to `\\n`.
//! The TypeScript `debugBody()` does *not* collapse newlines (only `debugTruncate()`
//! does). This has zero practical impact because both Rust call sites log JSON
//! output (`serde_json::to_string` and API error response bodies), and
//! `serde_json` never emits embedded newlines in its default compact output.

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Matches Claude Code's `DEBUG_MSG_LIMIT` at `project-zero/bridge/debugUtils.ts:9`.
///
/// Note: Claude Code measures this in UTF-16 code units (since JS `String.slice`
/// operates on code units). The Rust port measures in bytes. For ASCII-only
/// content the threshold is identical; for non-ASCII content the Rust port
/// truncates earlier (bytes >= chars). This is acceptable for diagnostic logging.
pub(crate) const DEBUG_MSG_LIMIT: usize = 2000;

/// Minimum length below which secret values are fully redacted instead of
/// partially shown. Matches `REDACT_MIN_LENGTH` in `project-zero/bridge/debugUtils.ts`.
const REDACT_MIN_LENGTH: usize = 16;

// ---------------------------------------------------------------------------
// Secret field names and compiled regex
// ---------------------------------------------------------------------------

/// Field names whose JSON values must be redacted in debug logs.
/// Order matches `project-zero` exactly. Order is not semantically significant
/// because the regex anchors on `"<field>"` so longer/shorter alternatives
/// cannot accidentally match each other's prefixes.
const SECRET_FIELD_NAMES: &[&str] = &[
    "session_ingress_token",
    "environment_secret",
    "access_token",
    "secret",
    "token",
];

static SECRET_PATTERN: Lazy<Regex> = Lazy::new(|| {
    let alts = SECRET_FIELD_NAMES.join("|");
    Regex::new(&format!(r#""({alts})"\s*:\s*"([^"]*)""#)).expect("valid secret regex")
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Replace secret field values with redacted placeholders.
///
/// Ports `redactSecrets` from `project-zero/bridge/debugUtils.ts:26-34`.
pub(crate) fn redact_secrets(s: &str) -> String {
    SECRET_PATTERN
        .replace_all(s, |caps: &Captures| {
            let field = &caps[1];
            let value = &caps[2];
            if value.len() < REDACT_MIN_LENGTH {
                format!(r#""{field}":"[REDACTED]""#)
            } else {
                let head_end = floor_boundary(value, 8);
                let tail_start = ceil_boundary(value, value.len().saturating_sub(4));
                let head = &value[..head_end];
                let tail = &value[tail_start..];
                format!(r#""{field}":"{head}...{tail}""#)
            }
        })
        .into_owned()
}

/// Truncate a string for debug logging, collapsing newlines and appending a
/// length suffix when truncated.
///
/// Ports `debugTruncate` from `project-zero/bridge/debugUtils.ts:37-43`.
pub(crate) fn debug_truncate(s: &str) -> String {
    let flat = s.replace('\n', "\\n");
    if flat.len() <= DEBUG_MSG_LIMIT {
        return flat;
    }
    let char_count = flat.chars().count();
    let mut end = DEBUG_MSG_LIMIT.min(flat.len());
    while end > 0 && !flat.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... ({char_count} chars)", &flat[..end])
}

/// Redact secrets then truncate for safe debug logging.
///
/// Ports `debugBody` from `project-zero/bridge/debugUtils.ts:46-53`.
///
/// Note: delegates to `debug_truncate`, so newlines are collapsed (unlike the
/// TypeScript `debugBody`). See the module-level documentation for rationale.
pub(crate) fn debug_body(raw: &str) -> String {
    let redacted = redact_secrets(raw);
    debug_truncate(&redacted)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Walk `idx` back to the nearest UTF-8 character boundary.
///
/// The helpers are defensive — real auth tokens are ASCII so byte indices 8 and
/// `len-4` are always char-safe — but if the regex ever matches a non-token
/// field (regression), the redaction path won't panic.
fn floor_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Walk `idx` forward to the nearest UTF-8 character boundary.
fn ceil_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Truncation tests (1-9)
    // -----------------------------------------------------------------------

    #[test]
    fn truncate_short_string_unchanged() {
        let output = debug_truncate("hello world");
        assert_eq!(output, "hello world");
    }

    #[test]
    fn truncate_exactly_at_limit() {
        let input = "x".repeat(DEBUG_MSG_LIMIT);
        let output = debug_truncate(&input);
        assert_eq!(output, input);
        assert!(!output.contains("..."), "no suffix when exactly at limit");
    }

    #[test]
    fn truncate_at_limit_plus_one() {
        let input = "x".repeat(DEBUG_MSG_LIMIT + 1);
        let output = debug_truncate(&input);
        let suffix = format!("... ({} chars)", DEBUG_MSG_LIMIT + 1);
        assert!(
            output.ends_with(&suffix),
            "expected suffix '{suffix}', got: {output}"
        );
        // First DEBUG_MSG_LIMIT bytes preserved (all ASCII, no boundary issues).
        assert!(output.starts_with(&"x".repeat(DEBUG_MSG_LIMIT)));
    }

    #[test]
    fn truncate_em_dash_at_boundary() {
        // 1998 ASCII bytes + "—" (3 bytes: 1998..2001).
        // Byte index 2000 lands inside the em-dash → must walk back to 1998.
        let mut input = "x".repeat(1998);
        input.push('\u{2014}'); // em-dash, 3 bytes
        assert_eq!(input.len(), 2001);
        let output = debug_truncate(&input);
        // Should truncate at byte 1998 (char boundary), not panic at byte 2000.
        let suffix = format!("... ({} chars)", 1999); // 1998 x's + 1 em-dash = 1999 chars
        assert!(
            output.ends_with(&suffix),
            "expected suffix '{suffix}', got: {output}"
        );
        assert!(output.starts_with(&"x".repeat(1998)));
        // The em-dash must NOT appear in the output (it starts at byte 1998, which is
        // where truncation occurred — the first 1998 bytes are all 'x').
    }

    #[test]
    fn truncate_two_byte_char_at_boundary() {
        // 1999 ASCII + é (2 bytes at 1999..2001). Byte 2000 mid-char → walk back to 1999.
        let mut input = "x".repeat(1999);
        input.push('\u{00E9}'); // é, 2 bytes
        assert_eq!(input.len(), 2001);
        let output = debug_truncate(&input);
        let suffix = format!("... ({} chars)", 2000); // 1999 x's + 1 é = 2000 chars
        assert!(
            output.ends_with(&suffix),
            "expected suffix '{suffix}', got: {output}"
        );
    }

    #[test]
    fn truncate_four_byte_char_at_boundary() {
        // 1997 ASCII + 🚀 (4 bytes at 1997..2001). Byte 2000 mid-char → walk back to 1997.
        let mut input = "x".repeat(1997);
        input.push('\u{1F680}'); // 🚀, 4 bytes
        assert_eq!(input.len(), 2001);
        let output = debug_truncate(&input);
        let suffix = format!("... ({} chars)", 1998); // 1997 x's + 1 rocket = 1998 chars
        assert!(
            output.ends_with(&suffix),
            "expected suffix '{suffix}', got: {output}"
        );
    }

    #[test]
    fn truncate_collapses_newlines() {
        let output = debug_truncate("foo\nbar\nbaz");
        assert_eq!(output, "foo\\nbar\\nbaz");
    }

    #[test]
    fn truncate_empty_string() {
        let output = debug_truncate("");
        assert_eq!(output, "");
    }

    #[test]
    fn truncate_single_byte_string() {
        let output = debug_truncate("a");
        assert_eq!(output, "a");
    }

    // -----------------------------------------------------------------------
    // Redaction tests (10-13)
    // -----------------------------------------------------------------------

    #[test]
    fn redact_short_token_full_redaction() {
        let input = r#"{"access_token":"abc"}"#;
        let output = redact_secrets(input);
        assert_eq!(output, r#"{"access_token":"[REDACTED]"}"#);
    }

    #[test]
    fn redact_long_token_partial() {
        let input = r#"{"access_token":"abcdefghijklmnopqrst"}"#;
        let output = redact_secrets(input);
        assert_eq!(output, r#"{"access_token":"abcdefgh...qrst"}"#);
    }

    #[test]
    fn redact_handles_all_field_names() {
        for field in SECRET_FIELD_NAMES {
            let json = format!(r#"{{"{field}":"my-secret-value-here"}}"#);
            let output = redact_secrets(&json);
            assert!(
                output.contains("[REDACTED]") || output.contains("..."),
                "field '{field}' was not redacted in: {output}"
            );
        }
    }

    #[test]
    fn redact_does_not_match_other_fields() {
        let input = r#"{"username":"foo"}"#;
        let output = redact_secrets(input);
        assert_eq!(output, input);
    }

    // -----------------------------------------------------------------------
    // Integration tests (14-15)
    // -----------------------------------------------------------------------

    #[test]
    fn debug_body_combines_redaction_and_truncation() {
        // Secret near the start so the [REDACTED] marker survives truncation.
        let input = format!(
            r#"{{"access_token":"short","padding":"{}"}}"#,
            "x".repeat(DEBUG_MSG_LIMIT)
        );
        let output = debug_body(&input);
        // The access_token value "short" is < 16 chars, so it should be [REDACTED].
        assert!(
            output.contains("[REDACTED]"),
            "secret should be redacted in combined output: {output}"
        );
        // The output should be truncated (input > DEBUG_MSG_LIMIT).
        assert!(
            output.contains("... ("),
            "output should have truncation suffix: {output}"
        );
    }

    #[test]
    fn debug_body_does_not_panic_on_em_dash_at_boundary() {
        // Explicit regression test: mirrors the production panic conditions.
        // An em-dash at byte positions 1999..2002 of the JSON body must not panic
        // when the full pipeline (redact_secrets → debug_truncate) processes it.
        let mut s = String::with_capacity(DEBUG_MSG_LIMIT + 10);
        // Build a JSON-like body prefix of ASCII content.
        s.push_str(&"x".repeat(1999));
        // Insert an em-dash so its byte span straddles the DEBUG_MSG_LIMIT boundary.
        s.push('\u{2014}'); // bytes 1999..2002
        s.push_str(&"x".repeat(8));
        // This must not panic.
        let output = debug_body(&s);
        // The suffix reports total char count (2000 x's + 1 em-dash + 8 x's = 2009 chars).
        let expected_suffix = format!("... ({} chars)", 1999 + 1 + 8);
        assert!(
            output.ends_with(&expected_suffix),
            "expected suffix '{expected_suffix}', got: {output}"
        );
    }
}
