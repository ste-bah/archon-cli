use archon_workflow::obligation_ids::{acceptance_ids, obligation_ids};

#[test]
fn obligations_union_req_bullets_and_obligation_tables() {
    let prd = r#"
## Requirements
- REQ-X-001: do the thing

## Acceptance Criteria
| ID | Criterion |
|---|---|
| AC-X-001 | accepted |
| OBL-X-002 | required behavior |

## Provider Matrix
| ID | Provider |
|---|---|
| PR-X-001 | Example |

## Non-goals
| ID | Criterion |
|---|---|
| AC-X-999 | excluded |
"#;
    let ids = obligation_ids(prd);
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        vec!["AC-X-001", "OBL-X-002", "REQ-X-001"]
    );
    assert_eq!(
        acceptance_ids(prd).into_iter().collect::<Vec<_>>(),
        vec!["AC-X-001"]
    );
}

#[test]
fn empty_prd_has_no_obligations_instead_of_a_synthetic_pass() {
    assert!(obligation_ids("").is_empty());
}

#[test]
fn residual_gap_forbidden_phrases_are_copied_from_the_prd_section() {
    let prd = r#"
## Explicit Residual-Gap Policy

Forbidden vague phrases:

- "should work"
- `later`
- best effort without fail-closed behavior

## Next Section
- "not part of the policy"
"#;
    assert_eq!(
        archon_workflow::obligation_ids::residual_gap_forbidden_phrases(prd),
        vec![
            "should work".to_string(),
            "later".to_string(),
            "best effort".to_string(),
        ]
    );
}

#[test]
fn acceptance_criteria_preserve_exact_prd_table_text() {
    let prd = r#"
## Acceptance Criteria
| ID | Acceptance criterion |
|---|---|
| AC-X-001 | `status` reports the exact project root. |
| AC-X-002 | Invalid input fails closed. |
"#;
    let criteria = archon_workflow::obligation_ids::acceptance_criteria(prd);
    assert_eq!(
        criteria.get("AC-X-001").map(String::as_str),
        Some("`status` reports the exact project root.")
    );
    assert_eq!(
        criteria.get("AC-X-002").map(String::as_str),
        Some("Invalid input fails closed.")
    );
}

#[test]
fn exact_obligation_grammar_rejects_numeric_areas_and_missing_areas() {
    let prd = r#"
## Requirements
- REQ-DL-040: valid
- REQ-DL2-041: numeric area is invalid
- REQ-042: missing area is invalid
- REQ-DL-043-extra: trailing slug is invalid

## Acceptance Criteria
| ID | Criterion |
|---|---|
| AC-DL-001 | valid |
| AC-DL2-002 | numeric area is invalid |
| AC-003 | missing area is invalid |
| AC-DL-006-extra | trailing slug is invalid |
| OBL-X-004 | valid generic obligation family |
| NFR-005 | valid legacy generic family without an area segment |
"#;

    assert_eq!(
        obligation_ids(prd).into_iter().collect::<Vec<_>>(),
        vec!["AC-DL-001", "NFR-005", "OBL-X-004", "REQ-DL-040"]
    );
    assert_eq!(
        archon_workflow::obligation_ids::malformed_obligation_ids(prd),
        vec![
            "AC-003",
            "AC-DL-006-extra",
            "AC-DL2-002",
            "REQ-042",
            "REQ-DL-043-extra",
            "REQ-DL2-041",
        ]
    );
}

#[test]
fn duplicate_acceptance_ids_are_reported_before_map_collapse() {
    let prd = r#"
## Acceptance Criteria
| ID | Criterion |
|---|---|
| AC-X-001 | first criterion |
| AC-X-001 | conflicting second criterion |
| AC-X-002 | valid criterion |
| AC-X-002 | |
"#;
    assert_eq!(
        archon_workflow::obligation_ids::duplicate_obligation_ids(prd),
        vec!["AC-X-001", "AC-X-002"]
    );
}
