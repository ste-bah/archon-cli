use std::fs;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin, FreezeGateMode,
    FreezeGateStamp, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::{
    FrozenDependency, FrozenTask, TaskSkeleton, TaskSkeletonLock, compare_frozen_task,
    validate_full_chain,
};
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;

fn clean_stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Enforce,
        finding_count: 0,
        findings_digest: archon_workflow::task_set_contract::empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-26T18:30:00Z".into(),
    }
}

fn write_acceptance(root: &std::path::Path) -> (String, AcceptancePin) {
    fs::create_dir_all(root).unwrap();
    let bytes = br#"{
      "schema_version":1,
      "prd":{"path":"prds/PRD-X.md","digest":"prd"},
      "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
      "acceptance":[{"id":"AC-X-001","criterion":"done","check":{"kind":"command","command":"sh -c 'exit 0'","cwd":"project_root"},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"attempted","reason":"rejects","host_call_id":"judge-1"}}],
      "supplementary":[]
    }"#;
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    let digest = content_digest(bytes);
    fs::write(
        root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: root.canonicalize().unwrap().display().to_string(),
        acceptance_digest: digest.clone(),
        freeze_event_id: "freeze-1".into(),
        acceptance_gate: clean_stamp(),
        skeleton_digest: None,
        skeleton_gate: None,
    };
    (digest, pin)
}

fn skeleton(acceptance_digest: &str) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.into(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: "TASK-X-010-body.md".into(),
            depends_on: vec![FrozenDependency {
                task_id: "TASK-X-001".into(),
                consumes: Vec::new(),
                ordering_only: true,
            }],
            blocks: Vec::new(),
            implements: vec!["AC-X-001".into()],
            deliverable_contracts: Vec::new(),
        }],
    }
}

#[test]
fn full_chain_binds_both_exact_files_to_the_host_pin() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let (acceptance_digest, mut pin) = write_acceptance(&root);
    let skeleton = skeleton(&acceptance_digest);
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    fs::write(root.join(TASK_SKELETON_FILE), &bytes).unwrap();
    let digest = content_digest(&bytes);
    fs::write(
        root.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest,
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    pin.skeleton_digest = Some(digest.clone());
    pin.skeleton_gate = Some(clean_stamp());
    assert_eq!(validate_full_chain(&root, &pin).unwrap(), skeleton);

    let mut reminted = skeleton.clone();
    reminted.tasks[0].implements.clear();
    let bytes = serde_json::to_vec_pretty(&reminted).unwrap();
    fs::write(root.join(TASK_SKELETON_FILE), &bytes).unwrap();
    fs::write(
        root.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: content_digest(&bytes),
            acceptance_digest: pin.acceptance_digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let error = validate_full_chain(&root, &pin).unwrap_err().to_string();
    assert!(error.contains(&digest), "{error}");
    assert!(error.contains("freeze-1"), "{error}");
    assert!(error.contains("restore the frozen version"), "{error}");
    assert!(error.contains("workflow freeze-skeleton"), "{error}");
}

#[test]
fn frozen_fields_compare_structurally_not_by_yaml_formatting() {
    let frozen = skeleton("acceptance").tasks.remove(0);
    let task = WorkflowV2TaskUniverseTask {
        canonical_task_id: frozen.task_id.clone(),
        source_path: format!("/tmp/{}", frozen.file_name),
        dependencies: frozen.depends_on.clone(),
        dependency_ids: vec!["TASK-X-001".into()],
        implements: frozen.implements.clone(),
        deliverable_contracts: frozen.deliverable_contracts.clone(),
        ..Default::default()
    };
    assert!(compare_frozen_task(&task, &frozen).is_empty());

    let mut dropped = task;
    dropped.implements.clear();
    let findings = compare_frozen_task(&dropped, &frozen);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].field, "implements");
    assert!(findings[0].message.contains("restore"));
}

#[test]
fn skeleton_set_validation_refuses_unowned_and_dangling_ids() {
    use archon_workflow::task_skeleton::validate_skeleton_set;
    use std::collections::BTreeSet;

    let mut value = skeleton("acceptance");
    let obligations: BTreeSet<_> = ["REQ-X-001".into(), "AC-X-001".into()]
        .into_iter()
        .collect();
    let findings = validate_skeleton_set(&value, &obligations);
    assert!(
        findings.iter().any(|finding| {
            finding.field == "implements"
                && finding.message.contains("REQ-X-001")
                && finding.message.contains("add it to at least one task")
        }),
        "{findings:?}"
    );
    assert!(
        findings.iter().any(|finding| {
            finding.field == "depends_on"
                && finding.message.contains("TASK-X-001")
                && finding.message.contains("add the missing task")
        }),
        "{findings:?}"
    );

    value.tasks.push(FrozenTask {
        task_id: "TASK-X-001".into(),
        file_name: "TASK-X-001-base.md".into(),
        depends_on: Vec::new(),
        blocks: vec!["TASK-X-010".into()],
        implements: vec!["REQ-X-001".into()],
        deliverable_contracts: Vec::new(),
    });
    assert!(validate_skeleton_set(&value, &obligations).is_empty());
}

#[test]
fn skeleton_set_validation_refuses_unknown_claims_but_allows_shared_ownership() {
    use archon_workflow::task_skeleton::validate_skeleton_set;
    use std::collections::BTreeSet;

    let mut value = skeleton("acceptance");
    value.tasks[0].depends_on.clear();
    value.tasks[0].implements = vec!["AC-X-001".into(), "REQ-X-999".into()];
    value.tasks.push(FrozenTask {
        task_id: "TASK-X-020".into(),
        file_name: "TASK-X-020-other.md".into(),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: vec!["AC-X-001".into()],
        deliverable_contracts: Vec::new(),
    });
    let obligations: BTreeSet<_> = ["AC-X-001".into()].into_iter().collect();
    let findings = validate_skeleton_set(&value, &obligations);
    assert!(
        findings.iter().any(|finding| {
            finding.message.contains("REQ-X-999") && finding.message.contains("remove")
        }),
        "{findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| finding.message.contains("AC-X-001")),
        "shared ownership is valid: {findings:?}"
    );
}

#[test]
fn skeleton_file_names_must_be_direct_task_markdown_names() {
    use archon_workflow::task_skeleton::validate_skeleton;

    for file_name in [
        "TASK-X-010/sub.md",
        "TASK-X-010\\sub.md",
        "TASK-X-010-body.txt",
        "OTHER-X-010.md",
    ] {
        let mut value = skeleton("acceptance");
        value.tasks[0].file_name = file_name.into();
        let error = validate_skeleton(&value, "acceptance")
            .unwrap_err()
            .to_string();
        assert!(error.contains(file_name), "{error}");
        assert!(error.contains("direct TASK-*.md filename"), "{error}");
    }
}

#[test]
fn skeleton_set_validation_refuses_cycles_and_contradictory_ordering() {
    use archon_workflow::task_skeleton::validate_skeleton_set;
    use std::collections::BTreeSet;

    let mut value = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "acceptance".into(),
        tasks: vec![
            FrozenTask {
                task_id: "TASK-X-001".into(),
                file_name: "TASK-X-001-base.md".into(),
                depends_on: vec![FrozenDependency {
                    task_id: "TASK-X-010".into(),
                    consumes: Vec::new(),
                    ordering_only: true,
                }],
                blocks: Vec::new(),
                implements: vec!["AC-X-001".into()],
                deliverable_contracts: Vec::new(),
            },
            FrozenTask {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010-body.md".into(),
                depends_on: vec![FrozenDependency {
                    task_id: "TASK-X-001".into(),
                    consumes: Vec::new(),
                    ordering_only: true,
                }],
                blocks: Vec::new(),
                implements: Vec::new(),
                deliverable_contracts: Vec::new(),
            },
        ],
    };
    let obligations: BTreeSet<_> = ["AC-X-001".into()].into_iter().collect();
    let findings = validate_skeleton_set(&value, &obligations);
    assert!(
        findings.iter().any(|finding| {
            finding.field == "dependency graph"
                && finding.message.contains("dependency cycle")
                && finding.message.contains("remove or reverse")
        }),
        "{findings:?}"
    );

    value.tasks[0].depends_on = vec![FrozenDependency {
        task_id: "TASK-X-010".into(),
        consumes: Vec::new(),
        ordering_only: true,
    }];
    value.tasks[0].blocks = vec!["TASK-X-010".into()];
    value.tasks[1].depends_on.clear();
    value.tasks[1].blocks.clear();
    let findings = validate_skeleton_set(&value, &obligations);
    assert!(
        findings.iter().any(|finding| {
            finding.field == "dependency graph"
                && finding.message.contains("both blocks and depends_on")
                && finding.message.contains("keep one direction")
        }),
        "{findings:?}"
    );
}

#[test]
fn dangling_blocks_target_is_a_finding_not_a_panic() {
    use archon_workflow::task_skeleton::validate_skeleton_set;
    use std::collections::BTreeSet;

    let mut value = skeleton("acceptance");
    value.tasks[0].depends_on.clear();
    value.tasks[0].blocks = vec!["TASK-X-999".into()];
    let obligations: BTreeSet<_> = ["AC-X-001".into()].into_iter().collect();

    let findings = std::panic::catch_unwind(|| validate_skeleton_set(&value, &obligations))
        .expect("dangling blocks must not panic");
    assert!(
        findings.iter().any(|finding| {
            finding.field == "blocks"
                && finding.message.contains("TASK-X-999")
                && finding
                    .message
                    .contains("add the missing task to the skeleton or remove the edge")
        }),
        "{findings:?}"
    );
}

#[test]
fn dangling_reference_does_not_hide_an_independent_cycle() {
    use archon_workflow::task_skeleton::validate_skeleton_set;
    use std::collections::BTreeSet;

    let value = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "acceptance".into(),
        tasks: vec![
            FrozenTask {
                task_id: "TASK-X-001".into(),
                file_name: "TASK-X-001-dangling.md".into(),
                depends_on: Vec::new(),
                blocks: vec!["TASK-X-999".into()],
                implements: vec!["AC-X-001".into()],
                deliverable_contracts: Vec::new(),
            },
            FrozenTask {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010-cycle.md".into(),
                depends_on: vec![FrozenDependency {
                    task_id: "TASK-X-020".into(),
                    consumes: Vec::new(),
                    ordering_only: true,
                }],
                blocks: Vec::new(),
                implements: Vec::new(),
                deliverable_contracts: Vec::new(),
            },
            FrozenTask {
                task_id: "TASK-X-020".into(),
                file_name: "TASK-X-020-cycle.md".into(),
                depends_on: vec![FrozenDependency {
                    task_id: "TASK-X-010".into(),
                    consumes: Vec::new(),
                    ordering_only: true,
                }],
                blocks: Vec::new(),
                implements: Vec::new(),
                deliverable_contracts: Vec::new(),
            },
        ],
    };
    let obligations: BTreeSet<_> = ["AC-X-001".into()].into_iter().collect();
    let findings = validate_skeleton_set(&value, &obligations);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("TASK-X-999")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("dependency cycle")),
        "{findings:?}"
    );
}
