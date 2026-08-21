//! Reading one item out of a markdown list.

/// The text of a markdown list item, whichever list marker was used.
///
/// Only `- ` and `* ` were accepted. Every task in the live PRD writes its
/// acceptance criteria as a numbered list, so all fifteen parsed to zero
/// criteria. `pin_noop_acceptance_criteria` then stamped an empty array, the
/// no-op proof rejected it as "acceptance_criteria is missing or empty", and
/// the repair loop could never satisfy a field no agent controls — twelve
/// identical iterations before the run halted.
pub(super) fn list_item_text(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return Some(rest.trim());
    }
    // `1. criterion`, `12) criterion` — the ordinal is the marker, not content.
    let digits = trimmed.len()
        - trimmed
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    if digits == 0 {
        return None;
    }
    let rest = &trimmed[digits..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .map(str::trim)
}
