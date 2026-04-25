//! Unknown-command diagnostic formatter.
//!
//! TASK-AGS-804 (ERR-SLASH-01 / EC-SLASH-01): dedicated formatter for
//! the "Unknown command" diagnostic emitted by
//! [`crate::command::dispatcher::Dispatcher`] when a slash name does
//! not resolve in the [`Registry`]. Replaces the ad-hoc inline format
//! string wired by TASK-AGS-803 with deterministic, spec-compliant
//! output and adds a case-insensitive exact-match fallback that was
//! absent from AGS-803.
//!
//! ## Contract
//!
//! Output rules (REQ-FOR-D7, validation criteria 1-5):
//!
//! 1. **Case-insensitive exact match** (checked BEFORE fuzzy) — if any
//!    primary name in `registry.names()` equals `name` under
//!    `eq_ignore_ascii_case`, emit
//!    `"Did you mean '/{actual}'? (commands are case-sensitive)"`.
//! 2. **Zero suggestions** → `"Unknown command '/{name}'. Type /help for the full list."`
//! 3. **One suggestion** → `"Unknown command '/{name}'. Did you mean '/{s1}'?"`
//! 4. **Two or three suggestions** →
//!    `"Unknown command '/{name}'. Did you mean one of: /{s1}, /{s2}, /{s3}?"`
//! 5. **Defensive truncation** — the formatter clips the suggestion
//!    list to at most 3 entries even if the upstream suggest()
//!    returns more (validation criterion 4). Belt-and-suspenders on
//!    top of `suggest(.., 3)`.
//!
//! The formatter is pure: no I/O, no channel sends. The dispatcher
//! owns the `TuiEvent::Error` emission; this module only produces the
//! message string.
//!
//! ## R-item summary (drift-reconcile against TASK-AGS-804.md)
//!
//! - R1 RELOCATION: spec `src/tui/command/errors.rs` shipped here as
//!   `src/command/errors.rs` (no `tui` subfolder in shipped tree;
//!   matches AGS-800..803 precedent).
//! - R2 RELOCATION: spec `tests/command_errors.rs` integration-test
//!   file shipped as `#[cfg(test)] mod tests` inline (matches AGS-800
//!   unit-test discipline; no new integration-test harness).
//! - R3 TYPE-DRIFT: spec `pub fn format_unknown_command` shipped as
//!   `pub(crate)` (binary crate, no out-of-tree consumers).
//! - R4 TYPE-DRIFT: spec `&CommandRegistry` shipped as `&Registry`
//!   (shipped type name; `CommandRegistry` does not exist).
//! - R5 BEHAVIOR-REWRITE: AGS-803 inline strings in dispatcher.rs
//!   replaced with this formatter's output; the two suggestion-branch
//!   dispatcher tests updated to match the new wording.
//! - R6 IMPROVEMENT: case-insensitive exact-match fallback (new
//!   semantic — AGS-803 had no such branch).
//! - R7 IMPROVEMENT: defensive truncation to 3 entries, independent
//!   of suggest()'s internal limit.
//! - R8 TEST-DESIGN: 6 inline unit tests (zero / one / two / three /
//!   four-truncated / case-diff).

use crate::command::parser;
use crate::command::registry::Registry;

/// Maximum number of suggestions surfaced to the user.
///
/// Kept as a `const` so the defensive-truncation assertion (validation
/// criterion 4) has a single source of truth shared with the test
/// suite.
pub(crate) const MAX_SUGGESTIONS: usize = 3;

/// Format the "Unknown command" diagnostic for `name` using the
/// registry's known primaries as a suggestion pool.
///
/// Dispatch order:
///
/// 1. Case-insensitive exact-match fallback wins first — if `name`
///    equals some registered primary under `eq_ignore_ascii_case`,
///    the user gets the case-sensitivity hint and fuzzy matching is
///    skipped.
/// 2. Otherwise fuzzy matching via
///    [`crate::command::parser::suggest`] with `limit = 3`.
/// 3. The final message is assembled by
///    [`format_from_suggestions`], which applies defensive
///    truncation on top of whatever `suggest()` returned.
///
/// Pure function; no I/O. See module docs for the full contract.
pub(crate) fn format_unknown_command(name: &str, registry: &Registry) -> String {
    // Step 1: case-insensitive exact-match fallback (spec: check
    // BEFORE fuzzy). Rationale: "/Model" should point the user at the
    // correctly-cased "/model" primary rather than offering a fuzzy
    // list that may or may not include it.
    let names = registry.names();
    if let Some(actual) = names.iter().find(|n| n.eq_ignore_ascii_case(name)).copied() {
        return format!("Did you mean '/{actual}'? (commands are case-sensitive)");
    }

    // Step 2: fuzzy match. `suggest(.., 3)` caps at 3 internally; the
    // formatter still truncates defensively (see
    // `format_from_suggestions`).
    let suggestions = parser::suggest(name, names.iter().copied(), MAX_SUGGESTIONS);
    format_from_suggestions(name, &suggestions)
}

/// Assemble the final message string given a pre-computed suggestion
/// list.
///
/// Exposed at `pub(crate)` so the defensive-truncation test can feed
/// in a 4-element list directly — `suggest(.., 3)` alone can never
/// produce one — and assert that the formatter clips it to
/// [`MAX_SUGGESTIONS`] regardless.
pub(crate) fn format_from_suggestions(name: &str, suggestions: &[String]) -> String {
    // Defensive truncation (R7 / validation criterion 4).
    let effective = if suggestions.len() > MAX_SUGGESTIONS {
        &suggestions[..MAX_SUGGESTIONS]
    } else {
        suggestions
    };

    match effective.len() {
        0 => format!("Unknown command '/{name}'. Type /help for the full list."),
        1 => format!(
            "Unknown command '/{name}'. Did you mean '/{}'?",
            effective[0]
        ),
        _ => {
            let joined = effective
                .iter()
                .map(|s| format!("/{s}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Unknown command '/{name}'. Did you mean one of: {joined}?")
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// Six unit tests per TASK-AGS-804 validation matrix (R8):
//
// 1. format_unknown_zero_suggestions_returns_help_hint
// 2. format_unknown_one_suggestion_returns_did_you_mean
// 3. format_unknown_two_suggestions_returns_one_of_list
// 4. format_unknown_three_suggestions_returns_one_of_list
// 5. format_unknown_four_suggestions_truncates_to_three
// 6. format_unknown_case_diff_returns_case_sensitive_hint
//
// Tests 1, 2, and 6 drive the full public formatter against a real
// `default_registry()` (the Gate-5 smoke path); tests 3, 4, and 5
// drive `format_from_suggestions` directly so they can precisely
// control suggestion cardinality — `suggest(.., 3)` caps at 3
// internally and the 37-command default registry does not produce
// deterministic 2- or 3-suggestion shapes for a chosen input.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;

    // -----------------------------------------------------------------
    // 1. Zero suggestions — "/help" hint string.
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_zero_suggestions_returns_help_hint() {
        // "zzzqqq" is > 2 edits from every registered primary, so
        // suggest() returns []. Expected: the "/help" hint exactly.
        let registry = default_registry();
        let msg = format_unknown_command("zzzqqq", &registry);
        assert_eq!(
            msg, "Unknown command '/zzzqqq'. Type /help for the full list.",
            "zero-suggestion path must emit the /help hint verbatim"
        );
    }

    // -----------------------------------------------------------------
    // 2. One suggestion — "Did you mean '/x'?" singular form.
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_one_suggestion_returns_did_you_mean() {
        // "hel" is 1 edit from "help" and > 2 from every other
        // primary in the default registry, so suggest() returns
        // ["help"] only. Expected: the singular "Did you mean '/help'?"
        // form.
        let registry = default_registry();
        let msg = format_unknown_command("hel", &registry);
        assert_eq!(
            msg, "Unknown command '/hel'. Did you mean '/help'?",
            "one-suggestion path must emit the singular 'Did you mean' form"
        );
    }

    // -----------------------------------------------------------------
    // 3. Two suggestions — "Did you mean one of: /a, /b?"
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_two_suggestions_returns_one_of_list() {
        // Drive `format_from_suggestions` directly so the test can
        // pin the suggestion cardinality without being at the mercy
        // of Levenshtein distances against the real registry.
        let suggestions = vec!["help".to_string(), "fast".to_string()];
        let msg = format_from_suggestions("hep", &suggestions);
        assert_eq!(
            msg, "Unknown command '/hep'. Did you mean one of: /help, /fast?",
            "two-suggestion path must use the 'one of' plural form"
        );
    }

    // -----------------------------------------------------------------
    // 4. Three suggestions — same plural form, three items.
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_three_suggestions_returns_one_of_list() {
        let suggestions = vec!["help".to_string(), "fast".to_string(), "fork".to_string()];
        let msg = format_from_suggestions("helf", &suggestions);
        assert_eq!(
            msg, "Unknown command '/helf'. Did you mean one of: /help, /fast, /fork?",
            "three-suggestion path must emit three comma-separated /name entries"
        );
    }

    // -----------------------------------------------------------------
    // 5. Four suggestions — defensive truncation to 3 (validation
    //    criterion 4). suggest() today caps at 3 internally, so the
    //    formatter's own .take(MAX_SUGGESTIONS) is belt-and-suspenders
    //    — this test exercises it directly by feeding 4 items in.
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_four_suggestions_truncates_to_three() {
        let suggestions = vec![
            "help".to_string(),
            "fast".to_string(),
            "fork".to_string(),
            "cost".to_string(),
        ];
        let msg = format_from_suggestions("helf", &suggestions);
        // Must NOT include the fourth suggestion.
        assert!(
            !msg.contains("/cost"),
            "formatter must drop the 4th suggestion, got: {msg}"
        );
        // Must include exactly the first three.
        assert_eq!(
            msg, "Unknown command '/helf'. Did you mean one of: /help, /fast, /fork?",
            "formatter must truncate to the first MAX_SUGGESTIONS entries"
        );
    }

    // -----------------------------------------------------------------
    // 6. Case-insensitive exact-match fallback (R6 / validation
    //    criterion 5).
    // -----------------------------------------------------------------

    #[test]
    fn format_unknown_case_diff_returns_case_sensitive_hint() {
        // "Help" equals "help" under eq_ignore_ascii_case. Expected:
        // the case-sensitivity hint, NOT the generic fuzzy "Did you
        // mean" form — and NO "Unknown command" prefix.
        let registry = default_registry();
        let msg = format_unknown_command("Help", &registry);
        assert_eq!(
            msg, "Did you mean '/help'? (commands are case-sensitive)",
            "case-diff path must point at the correctly-cased primary"
        );
    }
}
