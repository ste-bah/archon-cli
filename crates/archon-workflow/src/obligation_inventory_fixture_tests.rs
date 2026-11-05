//! Synthetic declarations only: no text copied from an external PRD.
use super::*;

fn fixture() -> String {
    let mut source = String::from(
        "# Product Requirements Document: Gadget archive\n## Goals\n| ID | Goal |\n|---|---|\n| G-X-001 | Find gadgets. |\n\n## Requirements\n",
    );
    for n in 1..=93 {
        source.push_str(&format!("- REQ-X-{n:03}: Store synthetic gadget {n}.\n"));
    }
    source.push_str("## Engineering Constraints\n- NFR-001: Bound gadget size.\n## Resolved Decisions\n- RD-001: Choose a gadget representation.\n\n## Acceptance Criteria\n| ID | Acceptance criterion |\n|---|---|\n| AC-X-001 | Retrieve a gadget. |\n| ordinary prose | acceptance must not change this table |\n| AC-X-002 | Reject an invalid gadget. |\n\n## Formats\n| ID | Format |\n|---|---|\n| TF-001 | compact |\n| ordinary prose | must not change the role |\n| TF-002 | expanded |\n\n## Provider Priority\n| Priority | Provider | Rule |\n|---|---|---|\n| P1 | dummy | must respond |\n| P2 | dummy | must respond |\n| P3 | dummy | must respond |\n| P4 | dummy | must respond |\n\n## Plan\n1. Plan only.\n2. Plan more.\n## Done Definition\n");
    for n in 1..=14 {
        source.push_str(&format!(
            "1. Finish gadget milestone {n}.\n   - Preserve its evidence.\n"
        ));
    }
    source.push_str("## Other\n1. Outside done.\nForbidden phrases:\n- `outside`\n## Residual Gap Policy\nForbidden phrases:\n- `later`\n## Following\n- `outside again`\n");
    source
}

#[test]
fn synthetic_archive_preserves_requirement_acceptance_goal_constraint_and_done_classes() {
    let source = fixture();
    let requirements = requirement_texts(&source);
    assert_eq!(requirements.len(), 93);
    assert_eq!(
        requirements.keys().cloned().collect::<BTreeSet<_>>(),
        (1..=93).map(|n| format!("REQ-X-{n:03}")).collect()
    );
    assert_eq!(
        acceptance_ids(&source),
        ["AC-X-001".into(), "AC-X-002".into()].into()
    );
    assert_eq!(
        constraint_ids(&source),
        ["NFR-001", "RD-001", "TF-001", "TF-002"]
            .map(String::from)
            .into()
    );
    let obligations = obligation_texts(&source);
    assert_eq!(obligations.len(), 93 + 2 + 1 + 14);
    assert!(obligations.contains_key("G-X-001"));
    for n in 1..=14 {
        assert_eq!(
            obligations[&format!("DONE-{n}")],
            format!("Finish gadget milestone {n}.; Preserve its evidence.")
        );
    }
    assert_eq!(residual_gap_forbidden_phrases(&source), ["later"]);
}

#[test]
fn table_role_and_id_column_are_fixed_by_header() {
    let source = "| Label | ID | Requirement |\n|---|---|---|\n| P1 | FR-001 | first |\n| must | ordinary prose | acceptance |\n| P2 | R1 | second |\n";
    let ids = requirement_texts(source)
        .into_keys()
        .collect::<BTreeSet<_>>();
    assert_eq!(ids, ["FR-001".into(), "R1".into()].into());
    let constraints = "| ID | Rule |\n|---|---|\n| ordinary prose | acceptance must be checked |\n| R1 | retain this constraint |\n";
    assert!(obligation_ids(constraints).is_empty());
    assert_eq!(constraint_ids(constraints), ["R1".into()].into());
}
