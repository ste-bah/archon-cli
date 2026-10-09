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
fn obligation_ids_and_texts_are_identical_for_crlf_prds() {
    let crlf = PRD.replace('\n', "\r\n");
    assert_eq!(obligation_ids(&crlf), obligation_ids(PRD));
    assert_eq!(obligation_texts(&crlf), obligation_texts(PRD));
    assert_eq!(acceptance_criteria(&crlf), acceptance_criteria(PRD));
}

#[test]
fn wrapped_requirement_text_stops_at_the_next_list_item() {
    let prd = "## Requirements\n\n- REQ-X-001: The first line\n  continues here\n- REQ-X-002: The sibling\n";
    let texts = obligation_texts(prd);
    assert_eq!(texts["REQ-X-001"], "The first line continues here");
    assert_eq!(texts["REQ-X-002"], "The sibling");
}

#[test]
fn lazy_unindented_requirement_continuation_is_included() {
    let prd = "- REQ-X-001: First line\ncontinues lazily\n- REQ-X-002: Next item\n";
    assert_eq!(
        obligation_texts(prd)["REQ-X-001"],
        "First line continues lazily"
    );
}

#[test]
fn table_rows_fences_and_html_comments_stop_continuations() {
    for blocker in [
        "  | AC-X-001 | table row |",
        "  ```rust",
        "  <!-- hidden comment -->",
    ] {
        let prd = format!("- REQ-X-001: Parent\n{blocker}\n  swallowed text\n");
        assert_eq!(obligation_texts(&prd)["REQ-X-001"], "Parent", "{blocker}");
    }
}

#[test]
fn nested_sub_bullet_prose_is_appended_but_identified_sub_bullets_are_separate() {
    let prose = "- REQ-X-001: Parent work\n  - also preserve the audit trail\n  - emit a summary\n";
    assert_eq!(
        obligation_texts(prose)["REQ-X-001"],
        "Parent work; also preserve the audit trail; emit a summary"
    );

    let identified = "- REQ-X-001: Parent work\n  - REQ-X-002: Separate work\n";
    let texts = obligation_texts(identified);
    assert_eq!(texts["REQ-X-001"], "Parent work");
    assert_eq!(texts["REQ-X-002"], "Separate work");
}

#[test]
fn requirement_text_excludes_nested_items_and_stops_at_blank_lines() {
    let prd = "## Requirements\n\n- REQ-X-001: Parent text\n  continuation\n  - REQ-X-002: Nested requirement\n  still nested\n\n  after blank\n- REQ-X-003: Before blank\n  included continuation\n\n  excluded continuation\n- REQ-X-004: Next requirement\n";
    let texts = obligation_texts(prd);
    assert_eq!(texts["REQ-X-001"], "Parent text continuation");
    assert_eq!(texts["REQ-X-002"], "Nested requirement still nested");
    assert_eq!(texts["REQ-X-003"], "Before blank included continuation");
    assert_eq!(texts["REQ-X-004"], "Next requirement");
}

#[test]
fn wrapped_requirement_is_crlf_safe_and_continuation_ids_are_not_obligations() {
    let prd = "## Requirements\r\n\r\n- REQ-X-001: Dataset uses provider data\r\n  REQ-X-099 is an identifier mentioned in continuation prose\r\n  with a final clause.\r\n";
    let texts = obligation_texts(prd);
    assert_eq!(
        texts["REQ-X-001"],
        "Dataset uses provider data REQ-X-099 is an identifier mentioned in continuation prose with a final clause."
    );
    assert!(!texts.contains_key("REQ-X-099"));
}

#[test]
fn a_markdown_heading_ends_requirement_continuations() {
    let prd =
        "- REQ-X-001: Before heading\n  included prose\n  Section title\n  ---\n  excluded prose\n";
    assert_eq!(
        obligation_texts(prd)["REQ-X-001"],
        "Before heading included prose"
    );
}

#[test]
fn wrapped_done_items_include_continuations_and_nested_sub_bullet_prose() {
    let prd = "## Done Definition\n\n1. First done statement\n   continues here\n   - nested detail\n     more detail\n2. Second done statement\n   finishes here\n";
    let texts = obligation_texts(prd);
    assert_eq!(
        texts["DONE-1"],
        "First done statement continues here; nested detail more detail"
    );
    assert_eq!(texts["DONE-2"], "Second done statement finishes here");
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

#[test]
fn done_section_nested_numbered_items_keep_main_ids() {
    let prd = "## Done Definition\n\n1. Parent\n   1. Nested numbered item\n2. Second parent\n";
    assert_eq!(obligation_ids(prd), old_obligation_ids(prd));
    assert_eq!(
        obligation_ids(prd),
        ["DONE-1", "DONE-2"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
}

/// Preserve the pre-change ID algorithm as a local oracle. The bullet and
/// table extractors are unchanged; DONE numbering intentionally mirrors main's
/// two-space indentation guard and section handling.
fn old_obligation_ids(prd: &str) -> BTreeSet<String> {
    let mut ids = bullet_requirement_ids(prd);
    ids.extend(table_obligation_ids(prd));
    let mut done_count = 0;
    let mut section_level: Option<usize> = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            let inside = section_level.is_some_and(|open| level > open);
            if !inside {
                section_level = heading_states_done(trimmed).then_some(level);
            }
            continue;
        }
        if section_level.is_none() || line.len() - trimmed.len() >= 2 {
            continue;
        }
        if numbered_item_text(trimmed).is_some() {
            done_count += 1;
            ids.insert(format!("{DONE_ITEM_PREFIX}{done_count}"));
        }
    }
    ids
}

#[test]
fn repository_prd_inputs_keep_the_same_ids_as_main() {
    let inputs = [
        PRD,
        include_str!("../../../tests/fixtures/decomposition-synthetic/prd.md"),
        include_str!("../../../tests/plan-reports/prd-trading-data-lake-ahdm-001.md"),
        include_str!("../../../assets/templates/workflow-prd.md"),
        include_str!("../../../assets/templates/workflow-prdtospec.md"),
        include_str!("../../../assets/templates/prdtospec.md"),
    ];
    for (index, input) in inputs.iter().enumerate() {
        assert_eq!(
            obligation_ids(input),
            old_obligation_ids(input),
            "PRD input {index}"
        );
    }
}
