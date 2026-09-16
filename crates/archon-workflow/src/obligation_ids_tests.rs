//! Extraction of goal tables, done-definition items and their exclusions.
//!
//! Every fixture here is a made-up PRD in a made-up domain: the extractor must
//! hold for any PRD, so nothing it is tested against may resemble a real one.

use super::*;

const PRD: &str = r#"
# Widget Service

## 3. Goals

| ID | Goal |
|---|---|
| G-WS-001 | Serve widgets from the existing store. |
| G-WS-002 | Ingest native widgets from every listed supplier. |

## 4. Non-Goals

| ID | Non-goal |
|---|---|
| G-WS-900 | Rewrite the store. |

## 5. Scope notes

| ID | Non-goal |
|---|---|
| G-WS-901 | this table sits under a neutral heading but its header says non-goal |

## 8. Requirements

- REQ-WS-001: Widgets are stored with provenance.
- REQ-WS-002 — Missing provenance fails closed.

## 12. Acceptance Criteria

| ID | Acceptance criterion |
|---|---|
| AC-WS-001 | Native widget ingestion stores a raw artifact and a registry entry. |

## 31. Done Definition

This PRD is done only when:

1. Existing store is upgraded, not replaced.
2. Native ingestion exists for all listed suppliers.
   1. an indented sub-item elaborates its parent and is not an obligation
3) A WidgetSpec exists and references registered widgets.

### 31.1 A sub-heading inside the section does not end it

4. Focused tests pass.

## 32. Residual gaps

1. this numbered line is outside the done section
"#;

#[test]
fn goal_table_rows_are_obligations_and_non_goals_are_not() {
    let ids = obligation_ids(PRD);
    assert!(ids.contains("G-WS-001"), "{ids:?}");
    assert!(ids.contains("G-WS-002"), "{ids:?}");
    assert!(
        !ids.contains("G-WS-900"),
        "a non-goals section is excluded by its heading: {ids:?}"
    );
    assert!(
        !ids.contains("G-WS-901"),
        "a header row naming a non-goal is excluded even under a neutral heading: {ids:?}"
    );
    assert!(
        malformed_obligation_ids(PRD).is_empty(),
        "single-letter goal families are well-formed: {:?}",
        malformed_obligation_ids(PRD)
    );
}

#[test]
fn done_items_become_positional_synthetic_obligations() {
    let ids = obligation_ids(PRD);
    let done: Vec<_> = ids
        .iter()
        .filter(|id| id.starts_with(DONE_ITEM_PREFIX))
        .cloned()
        .collect();
    assert_eq!(
        done,
        vec!["DONE-1", "DONE-2", "DONE-3", "DONE-4"],
        "{ids:?}"
    );
    let texts = obligation_texts(PRD);
    assert_eq!(
        texts.get("DONE-3").map(String::as_str),
        Some("A WidgetSpec exists and references registered widgets."),
        "the `3)` form counts and the indented sub-item does not shift numbering"
    );
    assert_eq!(
        texts.get("DONE-4").map(String::as_str),
        Some("Focused tests pass."),
        "a deeper sub-heading inside the done section does not end it"
    );
    assert!(
        !texts.contains_key("DONE-5"),
        "a numbered line after the next same-level heading is outside the section"
    );
}

#[test]
fn obligation_texts_carry_the_exact_statement_for_every_shape() {
    let texts = obligation_texts(PRD);
    assert_eq!(
        texts.get("REQ-WS-001").map(String::as_str),
        Some("Widgets are stored with provenance.")
    );
    assert_eq!(
        texts.get("REQ-WS-002").map(String::as_str),
        Some("Missing provenance fails closed."),
        "an em-dash separator is stripped like a colon"
    );
    assert_eq!(
        texts.get("G-WS-002").map(String::as_str),
        Some("Ingest native widgets from every listed supplier.")
    );
    assert_eq!(
        texts.get("AC-WS-001").map(String::as_str),
        Some("Native widget ingestion stores a raw artifact and a registry entry.")
    );
    assert_eq!(
        texts.keys().cloned().collect::<Vec<_>>(),
        obligation_ids(PRD).into_iter().collect::<Vec<_>>(),
        "every extracted id has a text and no text lacks an id"
    );
}

#[test]
fn done_headings_need_a_qualifier_and_respect_exclusions() {
    for heading in ["## Definition of Done", "## Done when", "## Done criteria"] {
        let prd = format!("{heading}\n\n1. first item\n");
        assert_eq!(
            obligation_ids(&prd).into_iter().collect::<Vec<_>>(),
            vec!["DONE-1"],
            "{heading}"
        );
    }
    for heading in ["## What we have done", "## Done items out of scope"] {
        let prd = format!("{heading}\n\n1. first item\n");
        assert!(
            obligation_ids(&prd).is_empty(),
            "{heading} must not mint done items"
        );
    }
}

#[test]
fn a_done_section_with_no_numbered_items_mints_nothing() {
    let prd = "## Done Definition\n\nProse only, no list.\n\n- a bullet is not a numbered item\n";
    assert!(obligation_ids(prd).is_empty());
    assert!(obligation_texts(prd).is_empty());
}
