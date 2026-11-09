#![allow(non_snake_case)]
use super::*;

fn skeleton() -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "digest".into(),
        tasks: vec![FrozenTask {
            requirement_states: Default::default(),
            proven_done: Default::default(),
            status: None,
            task_id: "TASK-X-001".into(),
            file_name: "TASK-X-001.md".into(),
            depends_on: vec![],
            blocks: vec![],
            implements: vec!["REQ-X-001".into()],
            source_sections: vec!["0".into()],
            deliverable_contracts: vec![],
        }],
    }
}
fn owners(s: &TaskSkeleton, prd: &str) -> Vec<TaskSetFinding> {
    constraint_owner_findings(s, &crate::obligation_ids::constraint_texts(prd))
}
#[test]
fn P5_1_unowned_constraint() {
    let findings = owners(&skeleton(), "- QX-001: bounded work\n");
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].identity,
        crate::defect::DeterministicDefect::new("unowned_constraint", "QX-001", "implements")
    );
    assert_eq!(
        findings[0].message,
        "PRD constraint 'QX-001' (bounded work) is in no task's implements; add it to the implements of every task whose work it restricts, or of at least one task when it restricts none directly"
    );
}
#[test]
fn P5_2_owned_constraint() {
    let mut s = skeleton();
    s.tasks[0].implements.push("QX-001".into());
    assert!(owners(&s, "- QX-001: bounded work\n").is_empty());
}
#[test]
fn P5_6_obligation_rule_unchanged() {
    let findings = validate_skeleton_set(&skeleton(), &["REQ-X-002".into()].into());
    assert!(
        findings
            .iter()
            .any(|f| f.identity.code == "unowned_obligation")
    );
}
#[test]
fn P5_7_obligation_takes_precedence() {
    let prd = "## Requirements\n| Requirement | Description |\n| --- | --- |\n| QX-001 | work |\n## Limits\n- QX-001: restrict work\n";
    assert!(crate::obligation_ids::obligation_ids(prd).contains("QX-001"));
    assert!(owners(&skeleton(), prd).is_empty());
}
#[test]
fn P5_8_no_constraints() {
    assert!(owners(&skeleton(), "- REQ-X-001: work\n").is_empty());
}
#[test]
fn P5_18_duplicate_entries() {
    for field in ["implements", "source_sections", "blocks", "depends_on"] {
        let mut s = skeleton();
        match field {
            "implements" => s.tasks[0].implements = vec!["REQ-X-001".into(); 2],
            "source_sections" => s.tasks[0].source_sections = vec!["0".into(); 2],
            "blocks" => s.tasks[0].blocks = vec!["TASK-X-002".into(); 2],
            _ => {
                s.tasks[0].depends_on = vec![
                    FrozenDependency {
                        task_id: "TASK-X-002".into(),
                        ordering_only: true,
                        consumes: vec![]
                    };
                    2
                ]
            }
        }
        let findings = validate_skeleton_set(&s, &BTreeSet::new());
        let f = findings
            .iter()
            .find(|f| f.identity.code == "duplicate_entry")
            .unwrap();
        assert_eq!(f.field, field);
        assert!(
            f.message
                .ends_with(&format!("twice in {field}; list each value once"))
        );
    }
}
#[test]
fn P5_19_untrimmed_entries() {
    for value in ["", " x", "x ", "\t"] {
        let mut s = skeleton();
        s.tasks[0].implements = vec![value.into()];
        s.tasks[0].source_sections = vec![value.into()];
        s.tasks[0].blocks = vec![value.into()];
        s.tasks[0].depends_on = vec![FrozenDependency {
            task_id: value.into(),
            ..Default::default()
        }];
        assert_eq!(
            entry_findings(&s)
                .iter()
                .filter(|f| f.identity.code == "untrimmed_entry")
                .count(),
            4
        );
    }
}
#[test]
fn P5_21_legacy_frozen_sets_normalize() {
    let mut frozen = skeleton().tasks.remove(0);
    frozen.implements = vec![" REQ-X-001 ".into(), "REQ-X-001".into(), "".into()];
    frozen.source_sections = vec!["0".into(), "0".into()];
    let body = WorkflowV2TaskUniverseTask {
        requirement_states: Default::default(),
        canonical_task_id: frozen.task_id.clone(),
        source_path: frozen.file_name.clone(),
        implements: vec!["REQ-X-001".into()],
        source_sections: vec!["0".into()],
        ..Default::default()
    };
    assert!(compare_frozen_task(&body, &frozen).is_empty());

    let mut findings = vec![];
    for field in ["blocks", "implements", "source_sections"] {
        compare_strings(
            &mut findings,
            field,
            vec!["x".into()],
            vec![" x ".into(), "x".into(), "".into()],
        );
    }
    let edge = FrozenDependency {
        task_id: "TASK-X-002".into(),
        ..Default::default()
    };
    compare_set(
        &mut findings,
        "depends_on",
        vec![edge.clone()],
        vec![edge.clone(), edge],
    );
    assert!(findings.is_empty());
}
#[test]
fn P5_22_dependency_identity_and_consumes() {
    let mut s = skeleton();
    s.tasks[0].depends_on = ["a", "b"]
        .map(|path| FrozenDependency {
            task_id: "TASK-X-002".into(),
            ordering_only: false,
            consumes: vec![ConsumedArtifact {
                artifact_path: path.into(),
                ..Default::default()
            }],
        })
        .into();
    assert_eq!(entry_findings(&s)[0].identity.code, "duplicate_entry");
    let mut findings = vec![];
    compare_set(
        &mut findings,
        "depends_on",
        vec![s.tasks[0].depends_on[0].clone()],
        s.tasks[0].depends_on.clone(),
    );
    assert_eq!(
        findings.len(),
        1,
        "different consumes are still different frozen dependencies"
    );
}

#[test]
fn offline_V1_skeleton_repairs_all_three_defects() {
    let prd = "## 1 Work\n- REQ-X-001: work\n- QX-002: bound work\n## 2 Closing\nGuidance.\n";
    let mut s = skeleton();
    s.tasks[0].source_sections = vec!["1".into(), "1".into()];
    let findings = |s: &TaskSkeleton| {
        let mut findings = validate_skeleton_set(s, &crate::obligation_ids::obligation_ids(prd));
        findings.extend(owners(s, prd));
        findings.extend(crate::obligation_ids::section_citation_findings(
            prd,
            s.tasks
                .iter()
                .map(|t| (t.task_id.as_str(), t.source_sections.as_slice())),
        ));
        findings
            .into_iter()
            .map(|f| (f.identity.code, f.identity.subject))
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        findings(&s),
        [
            ("duplicate_entry".into(), "TASK-X-001".into()),
            ("unowned_constraint".into(), "QX-002".into()),
            ("uncited_section".into(), "2".into())
        ]
        .into()
    );
    s.tasks[0].source_sections = vec!["1".into(), "2".into()];
    s.tasks[0].implements.push("QX-002".into());
    let mut allowed = crate::obligation_ids::obligation_ids(prd);
    allowed.extend(crate::obligation_ids::constraint_ids(prd));
    assert!(validate_skeleton_set(&s, &allowed).is_empty());
    assert!(owners(&s, prd).is_empty());
    assert!(
        crate::obligation_ids::section_citation_findings(
            prd,
            s.tasks
                .iter()
                .map(|t| (t.task_id.as_str(), t.source_sections.as_slice()))
        )
        .is_empty()
    );
}
