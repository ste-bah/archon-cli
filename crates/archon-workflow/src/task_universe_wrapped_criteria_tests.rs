//! A wrapped acceptance criterion must arrive whole.
//!
//! The parser kept only a bullet's first line, so every criterion long enough
//! to wrap reached the runtime cut off mid-sentence. This is the case that
//! proved it, quoted from a real task file: the clause naming what must be in
//! the registry, and the sentence refusing the exact shortcut that was taken,
//! both lived on the continuation lines and were discarded.

use super::*;

/// The real bullet, verbatim, wrapping over five lines exactly as authored.
const REAL_SECTION: &str = "\
## Acceptance Criteria

12. The ingest has been executed and `.archon/trading-lab/data/registry.json`
    under the selected project root lists a dataset for each of the thirty
    `trading-core-v1` cells, or a fail-closed unavailable record naming the exact
    reason for any cell a provider cannot supply. Compiling code with an empty
    data lake does not satisfy this task.
";

#[test]
fn a_wrapped_criterion_survives_whole() {
    let items = declared_task_section_items(REAL_SECTION, "acceptance criteria");
    assert_eq!(items.len(), 1, "one bullet, one item: {items:?}");
    let text = &items[0];

    assert!(
        text.contains("lists a dataset for each of the thirty"),
        "the clause naming what must be IN the registry was dropped: {text}"
    );
    assert!(
        text.contains("Compiling code with an empty data lake does not satisfy this task"),
        "the sentence refusing the shortcut was dropped: {text}"
    );
    assert!(
        !text.contains('\n'),
        "a wrap is joined, not preserved: {text}"
    );
}

/// Separate bullets stay separate — joining must not merge two criteria into
/// one, which would hide a requirement just as effectively as truncating it.
#[test]
fn separate_bullets_stay_separate() {
    let section = "\
## Acceptance Criteria

- First criterion, which wraps
  onto a second line.
- Second criterion.
";
    let items = declared_task_section_items(section, "acceptance criteria");
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(
        items
            .iter()
            .any(|i| i == "First criterion, which wraps onto a second line.")
    );
    assert!(items.iter().any(|i| i == "Second criterion."));
}

/// A blank line ends an item, so trailing prose is not swallowed into the last
/// bullet and silently reported as part of a criterion.
#[test]
fn trailing_prose_after_a_blank_line_is_not_swallowed() {
    let section = "\
## Acceptance Criteria

- Only criterion.

This closing paragraph is not a criterion.
";
    let items = declared_task_section_items(section, "acceptance criteria");
    assert_eq!(items, vec!["Only criterion.".to_string()], "{items:?}");
}

/// The section ends at the next heading, and an item open at that point is
/// still emitted rather than lost.
#[test]
fn an_item_open_at_the_next_heading_is_still_emitted() {
    let section = "\
## Acceptance Criteria

- Criterion that wraps
  across two lines.

## Files Expected to Change

- `src/lib.rs`
";
    let items = declared_task_section_items(section, "acceptance criteria");
    assert_eq!(
        items,
        vec!["Criterion that wraps across two lines.".to_string()]
    );
}
