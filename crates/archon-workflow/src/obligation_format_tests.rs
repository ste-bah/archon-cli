use super::*;

const STYLES: &[(&str, &str)] = &[
    ("REQ-X-001", "REQ-X-002"),
    ("FR-001", "FR-002"),
    ("R1", "R2"),
    ("REQ-1", "REQ-2"),
    ("US-12", "US-13"),
    ("feature_1", "feature_2"),
    ("FEATURE-login-1", "FEATURE-register-2"),
    ("Änderung.1", "Änderung.2"),
];

#[test]
fn declarations_have_equal_inventories_for_every_authored_style() {
    for &(first, second) in STYLES {
        for title in ["## 1 Requirements\n", "1 Other Requirements\n---\n"] {
            let source = format!("{title}\n- **{first}**: first\n  - `{second}`: second\n");
            assert_eq!(
                obligation_texts(&source),
                BTreeMap::from([
                    (first.into(), "first".into()),
                    (second.into(), "second".into()),
                ])
            );
            assert!(malformed_obligation_ids(&source).is_empty());
            assert_eq!(
                obligation_ids(&source),
                [first.into(), second.into()].into()
            );
            assert_eq!(
                obligation_source_sections(&source, &[second.into()]).len(),
                1
            );
        }
        let table = format!(
            "A different title\n---\n| Key | Requirement |\n|---|---|\n| **{first}** | first |\n| {second} | second |\n"
        );
        assert_eq!(
            obligation_texts(&table),
            BTreeMap::from([
                (first.into(), "first".into()),
                (second.into(), "second".into()),
            ])
        );
        assert_eq!(
            duplicate_obligation_ids(&format!("{table}| {first} | duplicate |\n")),
            [first]
        );
        let constraints = table.replace("Requirement", "Size rule");
        assert_eq!(
            constraint_ids(&constraints),
            [first.into(), second.into()].into()
        );
        assert!(obligation_ids(&constraints).is_empty());
    }
}

#[test]
fn emphasis_internal_punctuation_and_examples_do_not_change_declarations() {
    let source = "## Requirements\n- **R1:** first\n- `R2`: second\n- **R3** — third\n```text\n- R4: example\n```\n";
    assert_eq!(
        obligation_texts(source),
        BTreeMap::from([
            ("R1".into(), "first".into()),
            ("R2".into(), "second".into()),
            ("R3".into(), "third".into()),
        ])
    );
}

#[test]
fn completion_and_gap_declarations_respect_section_titles() {
    let source = "## Arbitrary\n1. ignored\n## Done Definition\n1. first\n2. second\n\n## Residual Gap Policy\nForbidden phrases:\n- `later`\n## Other\nForbidden phrases:\n- `outside`\n";
    assert_eq!(
        obligation_texts(source).into_values().collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(residual_gap_forbidden_phrases(source), ["later"]);
    let setext = source
        .replace("## Done Definition", "Done Definition\n---")
        .replace("## Residual Gap Policy", "Residual Gap Policy\n---");
    assert_eq!(obligation_texts(source), obligation_texts(&setext));
    assert_eq!(residual_gap_forbidden_phrases(&setext), ["later"]);
}

#[test]
fn setext_sections_have_the_same_keys_and_citation_findings_as_atx() {
    let atx = "# Source\n## 1 Requirements\nText.\n## 2 More\nOther text.\n```md\nFake\n---\n```\n";
    let setext = atx
        .replace("# Source", "Source\n======")
        .replace("## 1 Requirements", "1 Requirements\n---")
        .replace("## 2 More", "2 More\n---");
    let left = numbered_sections(atx);
    let right = numbered_sections(&setext);
    assert_eq!(
        left.keys().collect::<Vec<_>>(),
        right.keys().collect::<Vec<_>>()
    );
    for (key, section) in &left {
        assert_eq!(section.title, right[key].title);
        assert_eq!(section.requires_citation, right[key].requires_citation);
    }
    let atx_findings = section_citation_findings(atx, std::iter::empty());
    let setext_findings = section_citation_findings(&setext, std::iter::empty());
    assert_eq!(atx_findings.len(), setext_findings.len());
    for (a, b) in atx_findings.iter().zip(&setext_findings) {
        assert_eq!(a.identity, b.identity);
    }
}

#[test]
fn source_context_follows_declarations_instead_of_matching_prose() {
    let source = "## 1 Earlier\n- other text\n\n## 2 Requirements\n- R1: parent\n  - extra detail\n\n## 3 References\nR1: parent\n";
    assert_eq!(obligation_source_lines(source)["R1"], 5);
    let sections = obligation_source_sections(source, &["R1".into()]);
    assert_eq!(sections.len(), 1);
    assert!(sections[0].starts_with("## 2 Requirements"));
    assert!(sections[0].contains("extra detail"));
}

#[test]
fn acceptance_table_ids_are_authored_identities_too() {
    for id in ["AC-X-001", "FR-001", "R1", "US-12", "ITEM-1"] {
        let source = format!(
            "Autre titre\n---\n| Key | Criterion |\n|---|---|\n| **{id}** | Checked behavior. |\n"
        );
        assert_eq!(
            acceptance_criteria(&source),
            BTreeMap::from([(id.into(), "Checked behavior.".into())])
        );
    }
}

#[test]
fn no_separator_and_unparseable_declarations_are_visible() {
    let source = "## Requirements\n- REQ-X-001 Store gadgets\n- FR-2\n";
    assert_eq!(requirement_texts(source)["REQ-X-001"], "Store gadgets");
    assert_eq!(malformed_obligation_ids(source), ["FR-2"]);
}

#[test]
fn requirement_section_words_classify_every_id_family() {
    for title in [
        "Functional Spec",
        "FEATURES",
        "User Stories",
        "Requirements",
        "Functional Requirements",
    ] {
        let source = format!(
            "## 3 {title}\n- FR-1: first\n- R-2 second\n- NAMED-3: third\n\n| Identifier | Description |\n|---|---|\n| FR-4 | fourth |\n"
        );
        assert_eq!(requirement_texts(&source).len(), 4, "{title}");
        assert!(constraint_ids(&source).is_empty());
    }
}

#[test]
fn common_headers_and_first_column_adapter_keep_requirement_rows() {
    for header in [
        "ID",
        "Req ID",
        "requirement id",
        "#",
        "Identifier",
        "KEY",
        "Reference",
    ] {
        let source = format!(
            "## Requirements\n| {header} | Description |\n|---|---|\n| FR-1 | first |\n| no identity | cannot silently vanish |\n"
        );
        assert_eq!(requirement_texts(&source)["FR-1"], "first");
        assert_eq!(malformed_obligation_ids(&source).len(), 1);
    }
    let misplaced = "## Requirements\n| Reference | ID | Description |\n|---|---|---|\n| FR-2 | legacy label | second |\n";
    assert!(requirement_texts(misplaced).contains_key("FR-2"));
    let no_ids = "## Requirements\n| Label | Description |\n|---|---|\n| ordinary | missing id |\n";
    assert_eq!(malformed_obligation_ids(no_ids).len(), 1);
}

#[test]
fn titles_fields_and_nested_versions_never_inflate_inventory() {
    let source = "# Requirement planning\n## Data\n- dataset_id: field\n- max_retries: field\n- FR-2: constraint\n| Key | Value |\n|---|---|\n| dataset_id | field |\n| max_retries | field |\n## Requirements\n- FR-1: parent\n  - v2: nested version\n";
    assert_eq!(
        requirement_texts(source),
        BTreeMap::from([("FR-1".into(), "parent; v2: nested version".into())])
    );
    assert_eq!(constraint_ids(source), ["FR-2".into()].into());
    assert!(malformed_obligation_ids(source).is_empty());
    assert!(obligation_ids("## Requirements\n- feature-login: prose\n").is_empty());
}

#[test]
fn mixed_requirement_acceptance_tables_use_section_role_and_matching_text() {
    for (section, requirement) in [("Requirements", true), ("Acceptance Criteria", false)] {
        let source = format!(
            "## {section}\n| # | Requirement | Acceptance |\n|---|---|---|\n| FR-1 | required behavior | verified behavior |\n"
        );
        if requirement {
            assert_eq!(requirement_texts(&source)["FR-1"], "required behavior");
            assert!(acceptance_ids(&source).is_empty());
        } else {
            assert_eq!(acceptance_criteria(&source)["FR-1"], "verified behavior");
            assert!(requirement_texts(&source).is_empty());
        }
    }
}

#[test]
fn repeated_nested_version_labels_do_not_establish_an_id_family() {
    let source = "## Requirements\n- FR-1: parent\n  - v2: version two\n  - v3: version three\n";
    assert_eq!(
        requirement_texts(source),
        BTreeMap::from([(
            "FR-1".into(),
            "parent; v2: version two; v3: version three".into()
        )])
    );
    assert!(malformed_obligation_ids(source).is_empty());
}
