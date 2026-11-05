use super::*;

#[test]
fn source_rules_and_criteria_survive_heading_and_id_variants() {
    for id in ["REQ-X-001", "FR-001", "R1", "REQ-1", "US-12", "item_1"] {
        let atx = format!(
            "## Work\n- **{id}**: Preserve the input.\n## Constraints\n- Keep the record.\n"
        );
        let renamed = atx
            .replace("## Work", "Travail\n---")
            .replace("## Constraints", "Limites\n---");
        assert!(
            source_rules(&atx)
                .iter()
                .any(|text| text.contains("Preserve the input."))
        );
        assert!(
            source_rules(&renamed)
                .iter()
                .any(|text| text.contains("Keep the record."))
        );
        assert_eq!(acceptance_criteria(&atx), acceptance_criteria(&renamed));
    }
}

#[test]
fn unkeyed_list_intake_survives_section_renaming() {
    let original =
        "## Acceptance Criteria\n- Preserve the input.\n## Hard Rules\n- Keep the record.\n";
    let renamed = original
        .replace("## Acceptance Criteria", "Résultats\n---")
        .replace("## Hard Rules", "Limites\n---");
    assert_eq!(source_rules(original), source_rules(&renamed));
    assert_eq!(acceptance_criteria(original), acceptance_criteria(&renamed));
}

#[test]
fn table_rules_are_carried_without_heading_vocabulary() {
    let source = "Autre titre\n---\n| Key | Rule |\n|---|---|\n| R1 | Preserve the input. |\n";
    assert!(source_rules(source).contains(&"Preserve the input.".to_string()));
    assert!(acceptance_criteria(source).contains(&"Preserve the input.".to_string()));
}
