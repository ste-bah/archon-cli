use super::*;

const PRD: &str = "# Example\n## 7. Widgets\n---\n### 7.1 Small widgets\nRequired shapes.\n### 7.2 Empty leaf\n---\n## 8 Parent with body\nOverview.\n### 8.1 Detail\n- QX-001: Limit.\n## 9 Non-goals\nExcluded.\n### 9.1 Nested\nNever counted.\n## 10 Descriptive\nSome background.\n```md\n## 99 This is code\n```\n";

#[test]
fn own_bodies_parent_only_exclusions_empty_leaves_and_fences() {
    let sections = numbered_sections(PRD);
    assert_eq!(
        sections.keys().map(String::as_str).collect::<Vec<_>>(),
        ["10", "7", "7.1", "7.2", "8", "8.1"]
    );
    assert!(!sections["7"].requires_citation);
    for number in ["7.1", "7.2", "8", "8.1", "10"] {
        assert!(sections[number].requires_citation, "{number}");
    }
    assert!(!sections["8"].text.contains("Limit"));
    assert!(sections["10"].text.contains("## 99 This is code"));
    assert_eq!(sections, numbered_sections(&PRD.replace('\n', "\r\n")));
    assert_eq!(sections, numbered_sections(PRD));
    assert_eq!(
        numbered_sections("## 7x Not a number\ntext\n")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["0"]
    );
}

#[test]
fn old_skeleton_loads_and_missing_citations_are_repairable_set_findings() {
    let old: crate::task_skeleton::TaskSkeleton = serde_json::from_str(r#"{"schema_version":1,"acceptance_digest":"x","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":[] }]}"#).unwrap();
    assert!(old.tasks[0].source_sections.is_empty());
    let findings = section_citation_findings(
        PRD,
        old.tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task.source_sections.as_slice())),
    );
    assert_eq!(findings.len(), 5);
    assert!(
        findings
            .iter()
            .all(|finding| finding.identity.code == "uncited_section")
    );
    let numbers = ["7.1", "7.2", "8", "8.1", "10", "404"].map(String::from);
    let findings = section_citation_findings(PRD, [("TASK-X-001", numbers.as_slice())]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].identity.code, "unknown_section");
}

#[test]
fn unnumbered_offline_fixture_is_preamble_without_obligation_text_changes() {
    let prd = include_str!("../../../tests/fixtures/offline_decomposition/prd.md");
    let sections = numbered_sections(prd);
    assert_eq!(sections.len(), 1);
    assert_eq!(sections["0"].title, "Artifact Preparation");
    for text in ["## Objective", "## Requirements", "## Acceptance Criteria"] {
        assert!(sections["0"].text.contains(text));
    }
    assert_eq!(section_citation_findings(prd, std::iter::empty()).len(), 1);
    let before = serde_json::to_vec(&obligation_texts(prd)).unwrap();
    let _ = numbered_sections(prd);
    assert_eq!(before, serde_json::to_vec(&obligation_texts(prd)).unwrap());
}

#[test]
fn unnumbered_descendants_belong_to_nearest_numbered_ancestor() {
    let prd = "# Example\n## 7. Widgets\n### Details\nMust preserve widgets.\n#### More detail\n- Keep shapes.\n### 7.1 Sizes\nRequired size.\n#### Notes\nNever resize.\n### Non-goals\nExcluded text.\n## 8. Next\n";
    let sections = numbered_sections(prd);
    assert!(sections["7"].requires_citation);
    assert!(
        sections["7"]
            .text
            .contains("### Details\nMust preserve widgets.")
    );
    assert!(sections["7"].text.contains("Keep shapes."));
    assert!(!sections["7"].text.contains("Required size."));
    assert!(sections["7.1"].text.contains("Never resize."));
    assert!(
        !sections
            .values()
            .any(|section| section.text.contains("Excluded text."))
    );
}

#[test]
fn preamble_uses_zero_but_title_and_rules_alone_do_not() {
    for prd in ["# Example\n\n---\n## 7. Widgets\n", "# Example\n"] {
        assert!(!numbered_sections(prd).contains_key("0"));
    }
    let prd = "# Example\nIntro.\n## Context\nRequired context.\n## 7. Widgets\nBody.";
    let sections = numbered_sections(prd);
    assert_eq!(sections["0"].title, "Example");
    assert!(sections["0"].text.contains("Intro."));
    assert!(sections["0"].text.contains("Required context."));
    assert!(!sections["0"].text.contains("Body."));
    assert!(sections["0"].requires_citation);
    let explicit = numbered_sections("# Example\nIntro.\n## 0. Explicit\nMust keep both.");
    assert!(explicit["0"].text.contains("Intro."));
    assert!(explicit["0"].text.contains("Must keep both."));
}

#[test]
fn duplicate_numbers_combine_titles_and_all_texts() {
    let prd = "## 7. Widgets\nFirst body.\n## 7. Shapes\nSecond body.\n";
    let sections = numbered_sections(prd);
    assert_eq!(sections.len(), 1);
    assert_eq!(sections["7"].title, "Widgets / Shapes");
    assert!(sections["7"].text.contains("First body."));
    assert!(sections["7"].text.contains("Second body."));
    assert!(section_citation_findings(prd, [("TASK-X-001", ["7".into()].as_slice())]).is_empty());
}

#[test]
fn unnumbered_roots_after_numbered_sections_keep_their_exact_keys_and_children() {
    let prd = "# Example\n## 1. Intro\nBody one.\n## Appendix\nMust keep appendix text.\n### Details\nRequired detail.\n## Appendix ###\nNever drop the second occurrence.\n## Empty parent\n---\n### 2. Child\nBody two.\n## Non-goals\nExcluded appendix.\n### Notes\nExcluded child.\n";
    let sections = numbered_sections(prd);
    assert_eq!(
        sections.keys().map(String::as_str).collect::<Vec<_>>(),
        ["1", "2", "Appendix", "Empty parent"]
    );
    assert_eq!(sections["Appendix"].title, "Appendix / Appendix");
    for text in [
        "Must keep appendix text.",
        "### Details\nRequired detail.",
        "Never drop the second occurrence.",
    ] {
        assert!(sections["Appendix"].text.contains(text));
    }
    assert!(sections["Appendix"].requires_citation);
    assert!(!sections["Empty parent"].requires_citation);
    assert!(
        !sections
            .values()
            .any(|section| section.text.contains("Excluded"))
    );
    let cited = ["1", "2", "Appendix"].map(String::from);
    assert!(section_citation_findings(prd, [("TASK-X-001", cited.as_slice())]).is_empty());
    let excluded = ["Non-goals".to_string()];
    assert!(
        section_citation_findings(prd, [("TASK-X-001", excluded.as_slice())])
            .iter()
            .any(|finding| finding
                .message
                .contains("because the host excludes that section or a section it is under"))
    );
    let probe = numbered_sections(
        "# Example\n## 1. Intro\nBody one.\n## Appendix\nMust keep appendix text.\n",
    );
    assert_eq!(
        probe.keys().map(String::as_str).collect::<Vec<_>>(),
        ["1", "Appendix"]
    );
}

#[test]
fn excluded_numbers_are_defined_but_not_citable() {
    let numbers = ["9", "9.1", "404"].map(String::from);
    let findings = section_citation_findings(PRD, [("TASK-X-001", numbers.as_slice())]);
    for number in ["9", "9.1", "404"] {
        let finding = findings
            .iter()
            .find(|finding| finding.identity.location == format!("source_sections/{number}"))
            .unwrap();
        assert_eq!(finding.identity.code, "unknown_section");
        assert!(finding.message.contains(if number == "404" {
            "is not a section number the PRD defines"
        } else {
            "is not a citable PRD section because the host excludes that section or a section it is under"
        }));
    }
}
