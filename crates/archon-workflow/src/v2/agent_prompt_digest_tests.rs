use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "call-1".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-1".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

#[test]
fn reducer_prompt_uses_task_universe_digest_without_full_descriptions() {
    let mut request = request();
    request.call.id = "remediation-inventory-4".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "task_universe": {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[
                {
                    "canonical_task_id":"TASK-1",
                    "dependency_ids":[],
                    "source_path":"project-tasks/TASK-1.md",
                    "title":"dependency title must also be removed"
                },
                {
                    "canonical_task_id":"TASK-2",
                    "dependency_ids":["TASK-1"],
                    "source_path":"project-tasks/TASK-2.md",
                    "title":"large title that reducers do not need",
                    "acceptance_criteria":["large acceptance detail"],
                    "artifact_requirements":[".archon/artifacts/result.json"],
                    "deliverable_contracts":[{"kind":"json","artifact_path":"result.json"}]
                },
                {
                    "canonical_task_id":"TASK-9",
                    "dependency_ids":[],
                    "title":"unrelated task"
                }
            ]
        },
        "active":{"canonical_task_ids":["TASK-2"]}
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.stable_prefix.contains("TASK-2"));
    assert!(prompt.stable_prefix.contains("TASK-1"));
    assert!(
        prompt
            .stable_prefix
            .contains(".archon/artifacts/result.json")
    );
    assert!(!prompt.stable_prefix.contains("large title"));
    assert!(!prompt.stable_prefix.contains("large acceptance detail"));
    assert!(!prompt.stable_prefix.contains("deliverable_contracts"));
    assert!(!prompt.stable_prefix.contains("unrelated task"));
}

#[test]
fn canonical_inventory_prompt_keeps_full_task_universe() {
    let mut request = request();
    request.call.id = "canonical-implementation-inventory".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "task_universe": {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "source_path":"project-tasks/TASK-1.md",
                "title":"inventory needs full title",
                "acceptance_criteria":["inventory needs full acceptance"]
            }]
        }
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.stable_prefix.contains("inventory needs full title"));
    assert!(
        prompt
            .stable_prefix
            .contains("inventory needs full acceptance")
    );
}

#[test]
fn inventory_repair_call_families_keep_full_task_universe() {
    let prefixes = [
        "inventory-shape-repair",
        "task-universe-reconcile",
        "dependency-graph-repair",
        "target-file-discovery",
        "verification-requirements-discovery",
        "artifact-requirements-discovery",
        "provider-environment-discovery",
        "evidence-repair",
    ];
    for prefix in prefixes {
        let mut request = request();
        request.call.id = format!("{prefix}-1-transport-retry-2");
        request.call.method = WorkflowV2HostMethod::Reduce;
        request.input = serde_json::json!([{
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "title":format!("{prefix} full title"),
                "acceptance_criteria":["full acceptance"]
            }]
        }]);

        let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

        assert!(
            prompt
                .stable_prefix
                .contains(&format!("{prefix} full title")),
            "{prefix} lost its full authoritative task universe"
        );
    }
}

#[test]
fn non_universe_named_field_stays_in_invocation() {
    let mut request = request();
    request.call.id = "review-decoy-universe".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "task_universe":{"label":"domain metadata must remain"},
        "review":{"status":"pending"}
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("domain metadata must remain"));
    assert!(!prompt.stable_prefix.contains("domain metadata must remain"));
}

#[test]
fn top_level_array_reducer_receives_task_contract_context() {
    let mut request = request();
    request.call.id = "verification-failure-triage-2-1".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!([
        {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "acceptance_criteria":["array acceptance detail"],
                "deliverable_contracts":[{"kind":"json","artifact_path":"array.json"}]
            }]
        },
        {"triage":"input"}
    ]);

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("task_contract_context"));
    assert!(prompt.invocation.contains("array acceptance detail"));
}

#[test]
fn positional_evidence_with_marker_still_digests_old_wave_records() {
    let mut request = request();
    request.call.id = "completion-evidence-repair-5".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "source_data": [[
            {"kind":"noop-inventory-contradiction-reclassification","canonical_task_ids":["TASK-0"]},
            evidence_record(1, "marker-old-detail"),
            evidence_record(2, "marker-middle-detail"),
            evidence_record(3, "marker-latest-detail")
        ]]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(
        prompt
            .invocation
            .contains("noop-inventory-contradiction-reclassification")
    );
    assert!(!prompt.invocation.contains("marker-old-detail"));
    assert!(!prompt.invocation.contains("marker-middle-detail"));
    assert!(prompt.invocation.contains("marker-latest-detail"));
}

#[test]
fn transport_retry_ids_preserve_original_task_universe_policy() {
    let universe = serde_json::json!({
        "schema_version":"workflow-v2-task-universe-v1",
        "source_roots":["project-tasks"],
        "tasks":[{
            "canonical_task_id":"TASK-1",
            "title":"retry full title",
            "acceptance_criteria":["retry acceptance detail"],
            "deliverable_contracts":[{"kind":"json","artifact_path":"retry.json"}]
        }]
    });

    let mut inventory = request();
    inventory.call.id = "canonical-implementation-inventory-transport-retry-2".into();
    inventory.call.method = WorkflowV2HostMethod::Reduce;
    inventory.input = serde_json::json!([universe, {"discovery":"input"}]);
    let inventory_prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&inventory);
    assert!(inventory_prompt.stable_prefix.contains("retry full title"));

    let mut final_audit = request();
    final_audit.call.id = "final-zero-gap-audit-transport-retry-2".into();
    final_audit.call.method = WorkflowV2HostMethod::Reduce;
    final_audit.input = serde_json::json!([universe, {"evidence":"input"}]);
    let final_prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&final_audit);
    assert!(final_prompt.invocation.contains("retry acceptance detail"));
    assert!(final_prompt.invocation.contains("deliverable_contracts"));
}

#[test]
fn all_resultless_lifecycle_markers_preserve_bounded_evidence_history() {
    for marker in [
        "noop-proof-refutation-reclassification",
        "verification-supersede",
    ] {
        let mut request = request();
        request.call.id = "completion-evidence-repair-6".into();
        request.call.method = WorkflowV2HostMethod::Reduce;
        request.input = serde_json::json!({
            "source_data": [[
                {"kind":marker,"canonical_task_ids":["TASK-0"]},
                evidence_record(1, "extra-marker-old-detail"),
                evidence_record(2, "extra-marker-latest-detail")
            ]]
        });

        let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

        assert!(prompt.invocation.contains(marker));
        assert!(!prompt.invocation.contains("extra-marker-old-detail"));
        assert!(prompt.invocation.contains("extra-marker-latest-detail"));
    }
}

#[test]
fn remediation_reducers_receive_task_contract_context() {
    for call_id in [
        "remediation-inventory-2",
        "remediation-outcome-repair-2-1",
        "ownership-expansion-inventory-2-1",
    ] {
        let mut request = request();
        request.call.id = call_id.into();
        request.call.method = WorkflowV2HostMethod::Reduce;
        request.input = serde_json::json!([
            {
                "schema_version":"workflow-v2-task-universe-v1",
                "source_roots":["project-tasks"],
                "tasks":[{
                    "canonical_task_id":"TASK-1",
                    "acceptance_criteria":["remediation acceptance detail"],
                    "deliverable_contracts":[{"kind":"json","artifact_path":"repair.json"}]
                }]
            },
            {"failed":"input"}
        ]);

        let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

        assert!(
            prompt.invocation.contains("remediation acceptance detail"),
            "{call_id}"
        );
        assert!(
            prompt.invocation.contains("deliverable_contracts"),
            "{call_id}"
        );
    }
}

#[test]
fn older_record_digest_preserves_nested_branch_outcomes() {
    let mut request = request();
    request.call.id = "completion-evidence-repair-3".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "implementationEvidence": [
            {
                "kind":"implementation",
                "implementationWaveIndex":1,
                "result":{
                    "status":"needs_review",
                    "data":{"outcomes":[{
                        "item_id":"TASK-1-impl",
                        "canonical_task_ids":["TASK-1"],
                        "result":{"status":"failed","summary":"nested branch failed","private":"remove me"}
                    }]}
                }
            },
            evidence_record(2, "latest-full-detail")
        ]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("nested branch failed"));
    assert!(prompt.invocation.contains("TASK-1-impl"));
    assert!(prompt.invocation.contains("canonical_task_ids"));
    assert!(!prompt.invocation.contains("remove me"));
}

#[test]
fn final_reconciliation_and_reports_receive_task_contract_context() {
    for (method, call_id) in [
        (
            WorkflowV2HostMethod::Reduce,
            "final-evidence-reconciliation-2",
        ),
        (WorkflowV2HostMethod::Reduce, "final-zero-gap-audit"),
        (
            WorkflowV2HostMethod::FinalReport,
            "blocked-review-unresolved",
        ),
    ] {
        let mut request = request();
        request.call.id = call_id.into();
        request.call.method = method;
        request.input = serde_json::json!({
            "task_universe": {
                "schema_version":"workflow-v2-task-universe-v1",
                "source_roots":["project-tasks"],
                "tasks":[{
                    "canonical_task_id":"TASK-1",
                    "acceptance_criteria":["final acceptance detail"],
                    "deliverable_contracts":[{"kind":"json","artifact_path":"final.json"}]
                }]
            }
        });

        let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

        assert!(
            prompt.invocation.contains("final acceptance detail"),
            "{call_id}"
        );
        assert!(
            prompt.invocation.contains("deliverable_contracts"),
            "{call_id}"
        );
        assert!(
            !prompt.stable_prefix.contains("final acceptance detail"),
            "{call_id}"
        );
    }
}

#[test]
fn reducer_digest_membership_is_stable_across_active_tasks() {
    let universe = serde_json::json!({
        "schema_version":"workflow-v2-task-universe-v1",
        "source_roots":["project-tasks"],
        "tasks":[
            {"canonical_task_id":"TASK-1","dependency_ids":[]},
            {"canonical_task_id":"TASK-2","dependency_ids":["TASK-1"]}
        ]
    });
    let mut first = request();
    first.call.id = "remediation-inventory-1".into();
    first.call.method = WorkflowV2HostMethod::Reduce;
    first.input = serde_json::json!({
        "task_universe":universe,
        "active":{"canonical_task_ids":["TASK-1"]}
    });
    let mut second = first.clone();
    second.call.id = "remediation-inventory-2".into();
    second.input["active"]["canonical_task_ids"] = serde_json::json!(["TASK-2"]);

    let first = WorkflowV2AgentAdapter::new().build_prompt_parts(&first);
    let second = WorkflowV2AgentAdapter::new().build_prompt_parts(&second);

    assert_eq!(first.stable_prefix, second.stable_prefix);
    assert!(first.stable_prefix.contains("TASK-1"));
    assert!(first.stable_prefix.contains("TASK-2"));
}

#[test]
fn verification_reducer_keeps_full_acceptance_contracts() {
    let mut request = request();
    request.call.id = "verification-repair-plan-2-1".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "task_universe": {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "acceptance_criteria":["verification acceptance detail"],
                "deliverable_contracts":[{"kind":"json","artifact_path":"result.json"}]
            }]
        }
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("verification acceptance detail"));
    assert!(prompt.invocation.contains("deliverable_contracts"));
}

#[test]
fn task_universe_reconcile_attempt_keeps_full_task_universe() {
    let mut request = request();
    request.call.id = "task-universe-reconcile-2".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "task_universe": {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "source_path":"project-tasks/TASK-1.md",
                "title":"reconcile needs full title"
            }]
        }
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.stable_prefix.contains("reconcile needs full title"));
}

fn evidence_record(wave: usize, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "kind":"implementation",
        "implementationWaveIndex":wave,
        "dependencyIteration":wave,
        "readyImplementationItems":[{"item_id":format!("item-{wave}"),"detail":detail}],
        "result":{
            "status":"accepted",
            "summary":format!("wave {wave} accepted"),
            "outcomes":[{"item_id":format!("item-{wave}"),"status":"accepted","detail":detail}]
        }
    })
}
