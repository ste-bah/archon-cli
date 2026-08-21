//! Reading one item out of a markdown list, and the items under a heading.

use super::sorted_unique;

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

/// List items under a heading, each JOINED ACROSS THE LINES IT WRAPS ONTO.
///
/// A bullet that wraps is one item, and this used to keep only its first line:
/// a continuation line is not a list item, so it matched nothing and was
/// dropped in silence. Every criterion long enough to wrap reached the runtime
/// cut off mid-sentence.
///
/// That is not cosmetic. On a reference corpus one criterion read, in full:
/// "The ingest has been executed and `<registry>` ... lists a dataset for each
/// of the thirty cells ... Compiling code with an empty data lake does not
/// satisfy this task." The runtime saw it end at "`<registry>`". The clause
/// saying what must be IN the registry, and the sentence explicitly refusing
/// the exact shortcut that was taken, were both discarded before any gate could
/// read them — so runs closed the task on the strength of files existing, and
/// every stricter acceptance rule slid off a demand it could not see.
///
/// A blank line ends an item: it separates bullets in every markdown flavour,
/// and without it a trailing paragraph would be swallowed into the last one.
pub(super) fn declared_task_section_items(raw: &str, section: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut current: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            push_section_item(&mut items, current.take());
            in_section = heading
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.is_empty() {
            push_section_item(&mut items, current.take());
            continue;
        }
        if let Some(item) = list_item_text(trimmed) {
            push_section_item(&mut items, current.take());
            current = Some(item.to_string());
        } else if let Some(open) = current.as_mut() {
            // A continuation of the bullet above. Joined with a space because
            // the newline it replaces is a wrap, not a separator.
            open.push(' ');
            open.push_str(trimmed);
        }
    }
    push_section_item(&mut items, current);
    sorted_unique(items)
}

pub(super) fn push_section_item(items: &mut Vec<String>, item: Option<String>) {
    if let Some(item) = item {
        let item = item.trim();
        if !item.is_empty() {
            items.push(item.to_string());
        }
    }
}

#[cfg(test)]
#[path = "task_universe_wrapped_criteria_tests.rs"]
mod wrapped_criteria_tests;
