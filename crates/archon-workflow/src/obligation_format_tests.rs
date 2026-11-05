use super::*;

const STYLES: &[(&str, &str)] = &[
    ("REQ-X-001", "REQ-X-002"),
    ("FR-001", "FR-002"),
    ("R1", "R2"),
    ("REQ-1", "REQ-2"),
    ("US-12", "US-13"),
    ("feature_1", "feature_2"),
    ("feature-login", "feature-register"),
    ("Änderung.1", "Änderung.2"),
];

#[test]
fn declarations_have_equal_inventories_for_every_authored_style() {
    for &(first, second) in STYLES {
        for title in ["## 1 Work\n", "1 Autre titre\n---\n"] {
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
    let source = "- **R1:** first\n- `R2`: second\n- **R3** — third\n```text\n- R4: example\n```\n";
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
fn completion_and_gap_declarations_do_not_depend_on_section_titles() {
    let source =
        "## Arbitrary\n1. first\n2. second\n\n## Another title\nForbidden phrases:\n- `later`\n";
    assert_eq!(
        obligation_texts(source).into_values().collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(residual_gap_forbidden_phrases(source), ["later"]);
    let setext = source
        .replace("## Arbitrary", "Arbitrary\n---")
        .replace("## Another title", "Another title\n---");
    assert_eq!(obligation_texts(source), obligation_texts(&setext));
    assert_eq!(residual_gap_forbidden_phrases(&setext), ["later"]);
}

#[test]
fn setext_sections_have_the_same_keys_and_citation_findings_as_atx() {
    let atx = "# Source\n## 1 Work\nText.\n## 2 More\nOther text.\n```md\nFake\n---\n```\n";
    let setext = atx
        .replace("# Source", "Source\n======")
        .replace("## 1 Work", "1 Work\n---")
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
    let source = "## 1 Earlier\n- other text\n\n## 2 Work\n- R1: parent\n  - extra detail\n\n## 3 References\nR1: parent\n";
    assert_eq!(obligation_source_lines(source)["R1"], 5);
    let sections = obligation_source_sections(source, &["R1".into()]);
    assert_eq!(sections.len(), 1);
    assert!(sections[0].starts_with("## 2 Work"));
    assert!(sections[0].contains("extra detail"));
}

#[test]
fn acceptance_table_ids_are_authored_identities_too() {
    for id in ["AC-X-001", "FR-001", "R1", "US-12", "item_name"] {
        let source = format!(
            "Autre titre\n---\n| Key | Criterion |\n|---|---|\n| **{id}** | Checked behavior. |\n"
        );
        assert_eq!(
            acceptance_criteria(&source),
            BTreeMap::from([(id.into(), "Checked behavior.".into())])
        );
    }
}
