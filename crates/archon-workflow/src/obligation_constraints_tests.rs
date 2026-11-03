use super::*;

#[test]
fn generic_bullets_keep_continuations_and_exclude_obligations_and_nested_exclusions() {
    let prd = "# Contract\n- REQ-X-001: obligation\n- QX-001: first line\n  continued exactly.\n  - Ordinary detail.\n  - ZZ-002: independent\n+ QX-003: plus marker\n1. QX-004: numbered marker\n- QX-005: table obligation too\n\n| ID | Goal |\n|---|---|\n| QX-005 | goal |\n\n## Excluded\n- QX-006: excluded\n### Nested\n- QX-007: still excluded\n## Included\n* QX-008: included again\n- QX-001: duplicate loses\n- QX-0090: malformed\n- `QX-010`: not a leading id token\n";
    let texts = constraint_texts(prd);
    assert_eq!(
        texts["QX-001"],
        "first line continued exactly.; Ordinary detail."
    );
    assert_eq!(texts["ZZ-002"], "independent");
    assert_eq!(
        texts.keys().map(String::as_str).collect::<Vec<_>>(),
        ["QX-001", "QX-003", "QX-004", "QX-008", "ZZ-002"]
    );
    assert_eq!(constraint_ids(prd), texts.into_keys().collect());
    assert_eq!(
        constraint_texts(&prd.replace('\n', "\r\n")),
        constraint_texts(prd)
    );
}

#[test]
fn fourteen_id_prediction_uses_only_list_shape_not_section_names_or_prefixes() {
    let mut prd = String::from("# Example\n## Work\n- REQ-X-001: obligation\n## Limits\n");
    for number in 1..=6 {
        prd.push_str(&format!(
            "- QX-{number:03}: limit {number}\n  wrapped limit.\n"
        ));
    }
    prd.push_str("## Decisions\n");
    for number in 1..=8 {
        prd.push_str(&format!("- Z9-{number:03}: decision {number}\n"));
    }
    prd.push_str("## Non-goals\n- QX-099: excluded\n### Child\n- Z9-099: excluded child\n");
    let expected: BTreeSet<_> = (1..=6)
        .map(|n| format!("QX-{n:03}"))
        .chain((1..=8).map(|n| format!("Z9-{n:03}")))
        .collect();
    assert_eq!(constraint_ids(&prd), expected);
    assert_eq!(constraint_texts(&prd)["QX-003"], "limit 3 wrapped limit.");
}

#[test]
fn empty_map_for_requirement_only_input() {
    assert!(constraint_texts("- REQ-X-001: work\n").is_empty());
}

#[test]
fn offline_golden_prd_has_no_constraint_bullets() {
    let prd = include_str!("../../../tests/fixtures/offline_decomposition/prd.md");
    assert!(constraint_texts(prd).is_empty());
}

#[test]
fn only_definition_form_bullets_become_constraints() {
    let prd = "- SHA-256 digests pinned\n- HTTP-404 responses handled\n- AES-256 encryption used\n- TASK-X-001 owns work\n- AC-X-001 checks work\n- AC-X-001: acceptance owned elsewhere\n- REQ-001: malformed requirement\n- REQ-X-001: requirement\n- QX-001: text\n- QX-002 — dash text\n- QX-003 – other dash\n- QX-004:   \n- QX-005 - ordinary hyphen\n";
    assert_eq!(
        constraint_texts(prd),
        BTreeMap::from([
            ("QX-001".into(), "text".into()),
            ("QX-002".into(), "dash text".into()),
            ("QX-003".into(), "other dash".into()),
        ])
    );
    assert_eq!(malformed_obligation_ids(prd), vec!["REQ-001"]);
}

#[test]
fn nested_id_shaped_prose_stays_in_done_text() {
    let prd = "## Done definition\n1. Pin artifact digests.\n   - SHA-256 digests pinned\n   - HTTP-404 responses handled\n   - AES-256 encryption used\n";
    assert_eq!(
        obligation_texts(prd)["DONE-1"],
        "Pin artifact digests.; SHA-256 digests pinned; HTTP-404 responses handled; AES-256 encryption used"
    );
    assert!(constraint_texts(prd).is_empty());
}
