use super::*;

#[test]
fn all_seventy_invalid_filenames_are_reported_before_any_repair() {
    let mut skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "digest".into(),
        tasks: (1..=70)
            .map(|n| FrozenTask {
                task_id: format!("TASK-X-{n:03}"),
                file_name: format!("bad{n}"),
                depends_on: vec![],
                blocks: vec![],
                implements: vec![],
                deliverable_contracts: vec![],
            })
            .collect(),
    };
    for repaired in 0..70 {
        let error = validate_skeleton(&skeleton, "digest").expect_err("invalid filenames");
        assert_eq!(
            error
                .to_string()
                .matches("must be a direct TASK-*.md filename")
                .count(),
            70 - repaired,
            "every outstanding filename defect must be reported"
        );
        let task = &mut skeleton.tasks[repaired];
        task.file_name = format!("{}.md", task.task_id);
    }
    assert!(validate_skeleton(&skeleton, "digest").is_ok());
}

#[test]
fn independent_skeleton_header_and_task_defects_are_reported_together() {
    let skeleton = TaskSkeleton {
        schema_version: 9,
        acceptance_digest: "wrong".into(),
        tasks: vec![FrozenTask {
            task_id: "invalid".into(),
            file_name: "bad".into(),
            depends_on: vec![],
            blocks: vec![],
            implements: vec![],
            deliverable_contracts: vec![],
        }],
    };
    let message = validate_skeleton(&skeleton, "digest")
        .expect_err("invalid")
        .to_string();
    for part in [
        "schema_version",
        "acceptance_digest",
        "task_id",
        "file_name",
    ] {
        assert!(message.contains(part), "missing {part}: {message}");
    }
}

#[test]
fn acceptance_structure_reports_all_independent_entry_defects() {
    use crate::task_set_contract::{AcceptanceContract, validate_acceptance_structure};
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 9, "prd": {"path":"", "digest":""},
        "gap_policy": {"permitted_acceptance_ids": ["unknown"]},
        "acceptance": [{"id":"AC-X-001", "criterion":"", "check":{"kind":"command", "command":"", "cwd":"project_root"},
          "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":""}}],
        "supplementary": [{"id":"invalid", "criterion":"", "gap_permitted":true,
          "check":{"kind":"command", "command":"", "cwd":"project_root"},
          "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":""}}]
    })).expect("fixture");
    let expected = ["AC-X-001".into(), "AC-X-002".into()].into_iter().collect();
    let message = validate_acceptance_structure(&contract, &expected, true)
        .expect_err("invalid")
        .to_string();
    for part in [
        "schema_version",
        "unknown ids",
        "empty criterion",
        "counterexample",
        "host_call_id",
        "AC-X-002",
        "SUP-*",
        "residual gap",
    ] {
        assert!(message.contains(part), "missing {part}: {message}");
    }
}

#[test]
fn independent_graph_cycles_are_all_reported() {
    let tasks = (1..=4)
        .map(|n| FrozenTask {
            task_id: format!("TASK-X-{n:03}"),
            file_name: format!("TASK-X-{n:03}.md"),
            depends_on: vec![FrozenDependency {
                task_id: format!("TASK-X-{:03}", if n % 2 == 0 { n - 1 } else { n + 1 }),
                consumes: vec![],
                ordering_only: true,
            }],
            blocks: vec![],
            implements: vec![],
            deliverable_contracts: vec![],
        })
        .collect();
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "d".into(),
        tasks,
    };
    let findings = validate_skeleton_set(&skeleton, &BTreeSet::new());
    let message = findings
        .iter()
        .map(|f| f.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    for id in ["TASK-X-001", "TASK-X-002", "TASK-X-003", "TASK-X-004"] {
        assert!(message.contains(id), "unreported cycle: {message}");
    }
}

#[test]
fn all_independent_contract_template_paths_are_reported() {
    let value = serde_json::json!({"artifact_path":"${AUTHOR_A}/x", "registry_path":"${AUTHOR_B}/y",
        "instance_source_path":"${AUTHOR_C}/z"});
    let message = crate::v2::deliverable_contract::contract_defect(&value).expect("invalid paths");
    for field in ["artifact_path", "registry_path", "instance_source_path"] {
        assert!(message.contains(field), "missing {field}: {message}");
    }
}

#[test]
fn contract_audit_reports_all_path_and_verifier_defects_together() {
    use crate::task_universe::{
        WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
    };
    let contract = WorkflowV2DeliverableContract {
        artifact_path: "${AUTHOR_A}/x".into(),
        registry_path: Some("${AUTHOR_B}/y".into()),
        instance_source_path: Some("${AUTHOR_C}/z".into()),
        typed_verifier_command: Some("true".into()),
        ..Default::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "v1".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X-001".into(),
            deliverable_contracts: vec![contract],
            ..Default::default()
        }],
    };
    let findings = crate::task_universe_contract_audit::audit_contracts(&universe);
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.kind.is_certain())
            .count(),
        4,
        "three bad paths and the independently invalid verifier: {findings:?}"
    );
}

fn graph_task(n: u32, depends_on: &[u32], blocks: &[u32]) -> FrozenTask {
    let id = |n: &u32| format!("TASK-X-{n:03}");
    FrozenTask {
        task_id: id(&n),
        file_name: format!("{}.md", id(&n)),
        depends_on: depends_on
            .iter()
            .map(|n| FrozenDependency {
                task_id: id(n),
                consumes: vec![],
                ordering_only: true,
            })
            .collect(),
        blocks: blocks.iter().map(id).collect(),
        implements: vec![],
        deliverable_contracts: vec![],
    }
}

fn graph_codes(tasks: Vec<FrozenTask>) -> Vec<(String, String)> {
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "d".into(),
        tasks,
    };
    graph::graph_shape_findings(&skeleton)
        .into_iter()
        .map(|finding| (finding.identity.code, finding.message))
        .collect()
}

/// Issue 261 round 7: a contradiction is the authoring mistake; the
/// two-cycle it manufactures is not reported beside it.
#[test]
fn a_contradictory_pair_is_named_alone_without_its_manufactured_cycle() {
    let found = graph_codes(vec![graph_task(1, &[2], &[2]), graph_task(2, &[], &[])]);
    let codes: Vec<_> = found.iter().map(|(code, _)| code.as_str()).collect();
    assert_eq!(codes, ["contradictory_edge"], "{found:?}");
    let found = graph_codes(vec![graph_task(1, &[], &[2]), graph_task(2, &[], &[1])]);
    let codes: Vec<_> = found.iter().map(|(code, _)| code.as_str()).collect();
    assert_eq!(codes, ["mutual_blocks"], "one finding per pair: {found:?}");
}

#[test]
fn a_cycle_finding_names_the_cycle_path() {
    let found = graph_codes(vec![
        graph_task(1, &[2], &[]),
        graph_task(2, &[3], &[]),
        graph_task(3, &[1], &[]),
    ]);
    assert_eq!(
        found.len(),
        3,
        "one defect per task on the cycle: {found:?}"
    );
    for (code, message) in &found {
        assert_eq!(code, "dependency_cycle");
        for id in ["TASK-X-001", "TASK-X-002", "TASK-X-003"] {
            assert!(message.contains(id), "{message}");
        }
        assert!(message.contains(" -> "), "{message}");
    }
}
#[test]
fn dependency_declaration_identities_use_the_callers_subject() {
    let deps = vec![
        FrozenDependency {
            task_id: "TASK-X-002".into(),
            consumes: vec![],
            ordering_only: false,
        };
        2
    ];
    let findings =
        crate::task_set_edges::validate_dependency_declarations("TASK-X-001", "tasks/7", &deps);
    assert!(!findings.is_empty());
    assert!(
        findings
            .iter()
            .all(|finding| finding.identity.subject == "tasks/7"),
        "{findings:?}"
    );
}

/// Issue 261 round 8: a contradiction is named alone only for its own pair;
/// a cycle elsewhere in the graph is still reported beside it.
#[test]
fn a_contradiction_does_not_hide_an_independent_cycle() {
    let found = graph_codes(vec![
        graph_task(1, &[2], &[2]),
        graph_task(2, &[], &[]),
        graph_task(3, &[4], &[]),
        graph_task(4, &[3], &[]),
    ]);
    let codes: Vec<_> = found.iter().map(|(code, _)| code.as_str()).collect();
    assert_eq!(
        codes,
        ["contradictory_edge", "dependency_cycle", "dependency_cycle"],
        "{found:?}"
    );
    assert!(found[1].1.contains("TASK-X-003") && found[1].1.contains("TASK-X-004"));
}
