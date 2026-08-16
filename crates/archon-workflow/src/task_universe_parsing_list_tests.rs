use super::list_items::list_item_text;
use super::*;

/// The live failure. Every task in the PRD writes acceptance criteria as a
/// numbered list, so all fifteen parsed to zero criteria and the no-op proof
/// rejected each one as "acceptance_criteria is missing or empty".
#[test]
fn numbered_acceptance_criteria_are_parsed() {
    let raw = "\
# TASK-TDL-020

## Acceptance Criteria

1. Validation reports record every required check id.
2. Reports fail closed when evidence is absent.
12. Every filtered test runs at least one non-ignored test.

## Focused Tests
";

    let criteria = declared_task_section_items(raw, "acceptance criteria");

    assert_eq!(criteria.len(), 3, "got {criteria:?}");
    assert!(
        criteria
            .iter()
            .any(|c| c.starts_with("Validation reports record"))
    );
    assert!(
        criteria
            .iter()
            .any(|c| c.starts_with("Every filtered test runs"))
    );
    assert!(
        criteria.iter().all(|c| !c.starts_with(char::is_numeric)),
        "the ordinal is a marker, not content: {criteria:?}"
    );
}

/// Bullets must keep working — the PRD may mix styles.
#[test]
fn bullet_and_paren_markers_still_parse() {
    let raw = "\
## Acceptance Criteria

- Dash bullet criterion.
* Star bullet criterion.
+ Plus bullet criterion.
3) Paren-numbered criterion.

## Next
";

    let criteria = declared_task_section_items(raw, "acceptance criteria");

    assert_eq!(criteria.len(), 4, "got {criteria:?}");
    assert!(criteria.iter().any(|c| c == "Paren-numbered criterion."));
}

/// A section with no list yields nothing rather than swallowing prose.
#[test]
fn prose_without_list_markers_is_not_a_criterion() {
    let raw = "\
## Acceptance Criteria

This section is prose and declares no enumerated criteria.

## Next
";

    assert!(declared_task_section_items(raw, "acceptance criteria").is_empty());
}

/// A bare number is a marker only when a delimiter follows it.
#[test]
fn a_number_without_a_delimiter_is_not_a_list_item() {
    assert_eq!(list_item_text("2024 was the baseline year"), None);
    assert_eq!(list_item_text("1."), None);
    assert_eq!(list_item_text("1. real criterion"), Some("real criterion"));
}
