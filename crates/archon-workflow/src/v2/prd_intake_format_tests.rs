use super::*;

#[test]
fn intake_keeps_only_explicit_rules_and_criteria() {
    let source = "## Notes\n- ordinary note\n## Requirements\n- FR-001: work\n## Constraints\n- keep records\n## Acceptance Criteria\n- preserve input\n";
    assert_eq!(source_rules(source), ["keep records"]);
    assert_eq!(acceptance_criteria(source), ["preserve input"]);
}

#[test]
fn identifier_spelling_does_not_promote_notes_to_rules_or_criteria() {
    for id in ["REQ-X-001", "FR-001", "R1", "item_name"] {
        let source = format!("## Requirements\n- {id}: work\n## Notes\n- ordinary note\n");
        assert!(source_rules(&source).is_empty());
        assert!(acceptance_criteria(&source).is_empty());
    }
}
