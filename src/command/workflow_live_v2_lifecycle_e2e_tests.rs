use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_tui::event_channel::bounded_tui_event_channel;
use archon_workflow::{
    BranchFailureKind, WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter,
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::*;
use crate::command::workflow_live::workflow_live_task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use crate::command::workflow_live::workflow_live_v2::workflow_live_v2_verification;

struct CannedLifecycleLlm {
    scenario: CannedLifecycleScenario,
    calls: Mutex<Vec<String>>,
    deliverable_contract_executed: AtomicBool,
    parameterized_contract_executed: AtomicBool,
    inventory_calls: AtomicUsize,
    verification_failure_emitted: AtomicBool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CannedLifecycleScenario {
    #[default]
    FullLifecycle,
    TriagePreservation,
    RepairPlanPreservation,
    ZeroTestRetry,
    InventoryTombstone,
}

#[async_trait::async_trait]
impl LlmClient for CannedLifecycleLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("D64 lifecycle harness must use the agent execution seam")
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        let prompt = request
            .messages
            .iter()
            .filter_map(|message| message.get("content").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let call_id = prompt_line(&prompt, "call_id:").unwrap_or_else(|| request.task.clone());
        self.calls.lock().expect("calls lock").push(call_id.clone());
        let input = prompt_input(&prompt);

        let content = if self.scenario == CannedLifecycleScenario::TriagePreservation
            && call_id == "semantic-triage-shape-repair-1"
        {
            accepted_result(
                "shape repair accounted for more outcomes but rewrote a predicate",
                serde_json::json!({
                    "implementation_failures": [{
                        "item_id": "failed-one",
                        "source_item_id": "source-one",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "implementation_failure",
                        "failed_predicate": "mutated predicate",
                        "source_residual_gap_ids": ["gap-one"],
                    }],
                    "retry_items": [{
                        "item_id": "failed-two",
                        "source_item_id": "source-two",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "second predicate",
                        "source_residual_gap_ids": ["gap-two"],
                    }],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::RepairPlanPreservation
            && call_id == "post-remediation-verification-plan-repair-1-1-1"
        {
            accepted_result(
                "shape repair dropped source gap identity",
                serde_json::json!({
                    "items": [{
                        "item_id": "retry-check",
                        "source_item_id": "source-check",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "focused check passes",
                        "focused_verification": "cargo test focused_check -- --exact",
                    }],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::ZeroTestRetry
            && call_id == "zero-test-triage-shape-repair-1"
        {
            accepted_result(
                "zero-match verification routed to an informative retry",
                serde_json::json!({
                    "implementation_failures": [],
                    "retry_items": [{
                        "item_id": "zero-check",
                        "source_item_id": "source-zero-check",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "classification": "retryable_verification_shape_issue",
                        "failed_predicate": "the focused test must match at least one test",
                        "source_residual_gap_ids": ["zero_test_match_verification"],
                    }],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if self.scenario == CannedLifecycleScenario::InventoryTombstone
            && call_id == "inventory-shape-repair-1"
        {
            accepted_result(
                "inventory repair attempted to tombstone scheduled work",
                serde_json::json!({
                    "items": [{
                        "item_id": "scheduled-item",
                        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
                        "tombstone": true,
                    }],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "canonical-implementation-inventory" {
            if self.inventory_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                needs_review_result(
                    "malformed inventory exposes schedulable retry work",
                    serde_json::json!({
                        "items": [{"item_id": "malformed"}],
                        "retry_items": [{"item_id": "retry-inventory-generation"}],
                    }),
                )
            } else {
                accepted_result(
                    "synthetic inventory",
                    serde_json::json!({
                        "items": synthetic_inventory_items(),
                        "unresolved_issues": [],
                    }),
                    Vec::new(),
                    Vec::new(),
                )
            }
        } else if call_id.starts_with("inventory-shape-repair-") {
            needs_review_result(
                "inventory repair remains malformed with schedulable retry work",
                serde_json::json!({
                    "items": [{"item_id": "malformed"}],
                    "retry_items": [{"item_id": "retry-inventory-generation"}],
                }),
            )
        } else if call_id.starts_with("target-file-discovery-wave-") {
            let mut attempted_flip = implementation_item(
                "implementation-refuted-noop-refutable",
                "TASK-EX-002",
                "src/refuted.rs",
                "Refuted work is implemented.",
            );
            attempted_flip["work_type"] = serde_json::json!("verified_noop");
            accepted_result(
                "target discovery attempted to restore the demoted noop",
                serde_json::json!({
                    "items": [attempted_flip],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("noop-evidence-repair-") {
            accepted_result(
                "no safe noop proof repair exists",
                serde_json::json!({ "items": [], "unresolved_issues": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "verification-plan-1" {
            accepted_result(
                "focused verification plan",
                serde_json::json!({
                    "items": [
                        verification_item("verify-refuted", "TASK-EX-002", "src/refuted.rs"),
                        verification_item("verify-plain", "TASK-EX-003", "src/plain.rs"),
                        verification_item("verify-contract-source", "TASK-EX-004", "src/contract.rs"),
                        verification_item("verify-artifact-only", "TASK-EX-005", "../.archon/artifacts/artifact-only.json"),
                        verification_item("verify-parameterized", "TASK-EX-006", "src/parameterized.rs"),
                    ],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-failure-triage-")
            && call_id.ends_with("-shape-repair-1")
        {
            accepted_result(
                "shape repair accounts for the concrete failed outcome",
                serde_json::json!({
                    "implementation_failures": [{
                        "item_id": "verify-plain",
                        "source_item_id": "verify-plain",
                        "canonical_task_ids": ["TASK-EX-003"],
                        "classification": "implementation_failure",
                        "failure_status": "needs_review",
                        "failure_evidence": "the first focused check failed",
                        "required_fix": "re-apply the neutral implementation",
                    }],
                    "retry_items": [],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-failure-triage-") {
            accepted_result(
                "malformed triage omitted every canonical route",
                serde_json::json!({
                    "implementation_failures": [],
                    "retry_items": [],
                    "superseded_items": [],
                    "terminal_blockers": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("verification-remediation-inventory-") {
            accepted_result(
                "verification remediation inventory",
                serde_json::json!({
                    "items": [verification_remediation_item()],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("post-remediation-verification-plan-") {
            accepted_result(
                "post-remediation focused plan",
                serde_json::json!({
                    "items": [
                        verification_item("verify-refuted-remediated", "TASK-EX-002", "src/refuted.rs"),
                        verification_item("verify-plain-remediated", "TASK-EX-003", "src/plain.rs"),
                        verification_item("verify-contract-remediated", "TASK-EX-004", "src/contract.rs"),
                        verification_item("verify-artifact-remediated", "TASK-EX-005", "../.archon/artifacts/artifact-only.json"),
                        verification_item("verify-parameterized-remediated", "TASK-EX-006", "src/parameterized.rs"),
                    ],
                    "unresolved_issues": [],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "wave-completion-evidence-repair-1" {
            accepted_result(
                "wave completion is already represented by verification evidence",
                serde_json::json!({ "items": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "artifact-inventory" {
            accepted_result(
                "declared artifact inventory",
                serde_json::json!({
                    "artifacts": [{"path": ".archon/artifacts/example-contract.json"}],
                }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id.starts_with("adversarial-review-") {
            accepted_result(
                "synthetic adversarial review accepted",
                serde_json::json!({ "items": [] }),
                all_task_coverage(),
                vec![test_command("true", true, "review found no residual gaps")],
            )
        } else if call_id.starts_with("final-evidence-reconciliation-") {
            accepted_result(
                "final evidence reconciled",
                serde_json::json!({ "items": [] }),
                Vec::new(),
                Vec::new(),
            )
        } else if call_id == "final-zero-gap-audit" {
            accepted_result(
                "zero-gap audit accepted",
                serde_json::json!({ "items": [] }),
                all_task_coverage(),
                vec![test_command("true", true, "all synthetic checks passed")],
            )
        } else if request
            .task
            .contains("Re-verify repaired dependency-ready no-op proof")
            || request
                .task
                .contains("Verify the assigned dependency-ready no-op proof")
        {
            noop_proof_result(&call_id)
        } else if request
            .task
            .contains("Implement only the assigned dependency-ready item")
            || request
                .task
                .contains("Fix only the assigned focused-verification failure")
        {
            implementation_result(&request, &input, &call_id)?
        } else if request.task.contains("Run focused verification only")
            || request
                .task
                .contains("Run focused post-remediation verification only")
        {
            verification_result(
                &request,
                &input,
                &call_id,
                &self.deliverable_contract_executed,
                &self.parameterized_contract_executed,
                &self.verification_failure_emitted,
            )?
        } else if matches!(
            call_id.as_str(),
            "prd-task-review" | "repository-implementation-audit" | "acceptance-evidence-audit"
        ) {
            accepted_result(
                "synthetic read-only discovery",
                serde_json::json!({}),
                Vec::new(),
                Vec::new(),
            )
        } else {
            anyhow::bail!("unexpected D64 lifecycle harness call: {call_id}");
        };

        Ok(LlmResponse {
            content: content.to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_decomposed_lifecycle_normalizes_reclassified_ids_and_reaches_terminal() {
    let started = Instant::now();
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    for path in [
        "src/refuted.rs",
        "src/plain.rs",
        "src/contract.rs",
        "src/parameterized.rs",
    ] {
        std::fs::write(repo.join(path), "// pending\n").expect("seed source");
    }
    init_git_repo(&repo);
    std::fs::create_dir_all(temp.path().join(".archon/artifacts")).expect("artifact root");
    std::fs::write(
        temp.path().join(".archon/artifacts/example-contract.json"),
        r#"{"status":"ready","records":[{"id":"example","value":1}]}"#,
    )
    .expect("stub artifact");
    std::fs::write(
        temp.path().join(".archon/artifacts/instances.json"),
        r#"{"records":{}}"#,
    )
    .expect("empty instance source");

    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "d64-lifecycle-e2e".to_string(),
        task: "Run a neutral decomposed lifecycle fixture.".to_string(),
        target_repository_root: Some(repo.display().to_string()),
        max_parallelism: 4,
        max_agents: 16,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let llm = Arc::new(CannedLifecycleLlm {
        scenario: CannedLifecycleScenario::FullLifecycle,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        Some(repo.display().to_string()),
        Some(30),
    );
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: spec.target_repository_root.clone(),
        generated_config: archon_core::config::GeneratedWorkflowConfig {
            max_repair_iterations: 1,
            max_investigation_iterations: 1,
            verification_branch_timeout_secs: 30,
            host_call_timeout_secs: 30,
        },
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task.clone(),
        runtime,
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        Some(synthetic_task_universe(temp.path())),
        None,
    );

    let summary = tokio::time::timeout(
        Duration::from_secs(120),
        runner.run_decomposed_lifecycle(
            "# Archon decomposed-PRD workflow (native lifecycle e2e fixture)",
            serde_json::json!([]),
        ),
    )
    .await
    .expect("lifecycle harness timeout")
    .expect("lifecycle summary");

    assert_eq!(
        summary.status,
        WorkflowV2Status::Accepted,
        "failed_call={:?} next_action={:?} calls={:?} llm_calls={:?}",
        summary.failed_call,
        summary.next_action,
        summary
            .calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        llm.calls.lock().expect("calls lock"),
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("target-file-discovery-wave-1-"))
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id == "implementation-wave-1")
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id == "verification-wave-1"),
        "accepted implementation with a prefix-stripped ID was not scheduled for verification"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-failure-triage-")
                && call.id.ends_with("-shape-repair-1")),
        "empty triage routes did not trigger bounded shape repair"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-remediation-inventory-")),
        "shape-repaired triage route did not schedule remediation"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-wave-1-post-remediation-")),
        "remediation did not proceed to focused verification"
    );
    assert!(
        summary
            .calls
            .iter()
            .all(|call| !call.id.starts_with("blocked-"))
    );
    assert_eq!(
        summary
            .calls
            .iter()
            .filter(|call| call.id.starts_with("terminal-gate-reroute-"))
            .count(),
        1
    );
    assert_eq!(llm.inventory_calls.load(Ordering::SeqCst), 2);
    assert!(
        llm.deliverable_contract_executed.load(Ordering::SeqCst),
        "declared deliverable contract verification command did not execute"
    );
    assert!(
        llm.parameterized_contract_executed.load(Ordering::SeqCst),
        "vacuous source-backed parameterized contract did not traverse lifecycle verification"
    );
    assert!(
        std::fs::read_to_string(repo.join("src/refuted.rs"))
            .expect("refuted implementation")
            .contains("implemented_TASK_EX_002")
    );
    assert!(
        temp.path()
            .join(".archon/artifacts/artifact-only.json")
            .is_file()
    );

    let final_record = v2_store
        .load_call_record("final-acceptance-report")
        .expect("final record load")
        .expect("final record");
    assert_eq!(final_record.status, WorkflowV2Status::Accepted);
    let report = &final_record.result.data;
    assert_eq!(report["accepted_tasks"].as_array().map(Vec::len), Some(5));
    assert_eq!(report["noop_tasks"], serde_json::json!(["TASK-EX-001"]));
    assert!(report["failed_tasks"].as_array().is_some_and(Vec::is_empty));
    assert!(
        report["blocked_tasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(
        report["missing_tasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "harness took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_final_report_emits_host_built_fallback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "final-report-fallback-e2e".to_string(),
        task: "Force the terminal report fallback path.".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-FALLBACK".to_string(),
            source_path: "tasks/TASK-EX-FALLBACK.md".to_string(),
            acceptance_criteria: vec!["A terminal report is persisted.".to_string()],
            ..Default::default()
        }],
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let llm = Arc::new(CannedLifecycleLlm {
        scenario: CannedLifecycleScenario::FullLifecycle,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(llm, tui_tx, Vec::new(), run.id.clone(), None, Some(30));
    let generated_config = archon_core::config::GeneratedWorkflowConfig {
        max_repair_iterations: 1,
        max_investigation_iterations: 1,
        verification_branch_timeout_secs: 30,
        host_call_timeout_secs: 30,
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task,
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: generated_config.clone(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        Some(universe.clone()),
        None,
    );
    let host = Arc::new(WorkflowScriptHost {
        scaffold_hash: workflow_scaffold_hash("# final report fallback fixture"),
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
    });
    let driver = LifecycleDriver::new(
        host,
        universe,
        None,
        Some(temp.path().display().to_string()),
        serde_json::json!([]),
        Default::default(),
        &generated_config,
    );

    let result = driver
        .final_report(
            "forced-report-failure",
            None,
            "needs_review",
            serde_json::json!([{
                "status": "failed",
                "summary": "forced malformed reducer result",
                "commands_run": "not-a-sequence"
            }]),
            "Emit a terminal report even when reducer evidence is malformed.",
        )
        .await;

    assert!(
        result
            .expect_err("fallback report should terminate needs-review lifecycle")
            .to_string()
            .contains(TERMINAL_HOST_CALL_MARKER)
    );
    assert_eq!(
        v2_store
            .load_call_record("forced-report-failure")
            .expect("failed report record load")
            .expect("failed report record")
            .status,
        WorkflowV2Status::Failed
    );
    let fallback = v2_store
        .load_call_record("forced-report-failure-host-fallback")
        .expect("fallback record load")
        .expect("fallback record");
    assert_eq!(fallback.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        fallback.result.data["missing_tasks"],
        serde_json::json!(["TASK-EX-FALLBACK"])
    );
    assert!(fallback.result.artifacts.iter().any(|artifact| {
        artifact.id == "forced-report-failure-host-fallback"
            && std::path::Path::new(&artifact.path).is_file()
    }));
}

#[tokio::test]
async fn triage_shape_repair_cannot_trade_predicate_identity_for_better_accounting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::TriagePreservation);
    let original = serde_json::json!({
        "implementation_failures": [{
            "item_id": "failed-one",
            "source_item_id": "source-one",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "classification": "implementation_failure",
            "failed_predicate": "original predicate",
            "source_residual_gap_ids": ["gap-one"],
        }],
        "retry_items": [],
        "superseded_items": [],
        "terminal_blockers": [],
    });
    let failed_outcomes = vec![
        serde_json::json!({"item_id": "failed-one"}),
        serde_json::json!({"item_id": "failed-two"}),
    ];
    let mut evidence = LifecycleEvidence::default();

    let retained = driver
        .enforce_triage_accounting(
            "semantic-triage",
            &failed_outcomes,
            original.clone(),
            &mut evidence,
        )
        .await
        .expect("triage repair");

    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&retained);
    assert_eq!(routes.implementation_failures.len(), 1);
    assert!(routes.retry_items.is_empty());
    assert_eq!(
        routes.implementation_failures[0]["failed_predicate"],
        "original predicate"
    );
    assert!(evidence.repair_attempts.iter().any(|attempt| {
        attempt["call_id"] == "semantic-triage-shape-repair-1"
            && attempt["issue_kind"] == "semantic_preservation_rejected"
    }));
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["semantic-triage-shape-repair-1"]
    );
    // D78: the rejection must persist as a monitor-visible typed record, not
    // only in in-memory repair-attempt evidence.
    assert!(
        persisted_semantic_rejection_record(temp.path(), "semantic-triage-shape-repair-1"),
        "expected a persisted semantic-preservation rejection record"
    );
}

fn persisted_semantic_rejection_record(root: &std::path::Path, repair_id: &str) -> bool {
    fn walk(dir: &std::path::Path, needle: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path, needle) {
                    return true;
                }
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(needle))
            {
                return true;
            }
        }
        false
    }
    walk(root, &format!("{repair_id}-semantic-preservation-rejected"))
}

#[tokio::test]
async fn repair_plan_shape_repair_cannot_drop_source_gap_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::RepairPlanPreservation);
    let post_plan = serde_json::json!({
        "items": [{
            "item_id": "retry-check",
            "source_item_id": "source-check",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "classification": "retryable_verification_shape_issue",
            "failed_predicate": "focused check passes",
            "source_residual_gap_ids": ["gap-check"],
            "focused_verification": "cargo test focused_check -- --exact",
        }],
        "unresolved_issues": [{
            "kind": "inventory_shape_repair",
            "field": "items",
            "message": "repair the plan shape",
        }],
    });
    let mut evidence = LifecycleEvidence::default();

    let retained = driver
        .repair_post_remediation_plan_once(
            &serde_json::json!({"items": []}),
            &serde_json::json!({"outcomes": []}),
            post_plan,
            1,
            &1,
            1,
            &mut evidence,
        )
        .await
        .expect("post-remediation plan repair");

    assert_eq!(
        retained["items"][0]["source_residual_gap_ids"],
        serde_json::json!(["gap-check"])
    );
    assert!(
        retained["unresolved_issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| {
                issue["kind"] == "semantic_preservation"
                    && issue["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("source_residual_gap_ids"))
            }))
    );
    assert!(evidence.repair_attempts.iter().any(|attempt| {
        attempt["call_id"] == "post-remediation-verification-plan-repair-1-1-1"
            && attempt["issue_kind"] == "semantic_preservation_rejected"
    }));
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["post-remediation-verification-plan-repair-1-1-1"]
    );
}

#[tokio::test]
async fn accepted_zero_match_verification_is_demoted_and_routed_to_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::ZeroTestRetry);
    let mut result = WorkflowV2Result::accepted("focused verification claimed acceptance");
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test missing_check -- --exact".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary:
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out"
                .to_string(),
    });
    result.data = serde_json::json!({
        "source_item_id": "source-zero-check",
        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
    });
    let mut outcome = WorkflowV2BranchOutcome {
        item_id: "zero-check".to_string(),
        role: "verifier".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some("zero-test-input".to_string()),
        completion_evidence: Vec::new(),
    };
    workflow_live_v2_verification::normalize_focused_verification_outcome(
        "verification-wave-1-1",
        &mut outcome,
    );

    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
    assert_eq!(
        outcome.result.as_ref().expect("result").data["zero_test_match"],
        true
    );

    let failed_outcomes = vec![serde_json::to_value(&outcome).expect("serialize outcome")];
    let mut evidence = LifecycleEvidence::default();
    let triage = driver
        .enforce_triage_accounting(
            "zero-test-triage",
            &failed_outcomes,
            serde_json::json!({
                "implementation_failures": [],
                "retry_items": [],
                "superseded_items": [],
                "terminal_blockers": [],
            }),
            &mut evidence,
        )
        .await
        .expect("zero-test triage repair");

    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&triage);
    assert_eq!(routes.retry_items.len(), 1);
    assert_eq!(
        routes.retry_items[0]["classification"],
        "retryable_verification_shape_issue"
    );
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["zero-test-triage-shape-repair-1"]
    );
}

#[tokio::test]
async fn inventory_repair_tombstone_cannot_remove_scheduled_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::InventoryTombstone);
    let inventory = serde_json::json!({
        "items": [{
            "item_id": "scheduled-item",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "dependency_ids": [],
            "target_files": ["src/boundary.rs"],
            "acceptance_criteria": ["Scheduled work survives inventory repair."],
            "focused_verification": "cargo test boundary_check -- --exact",
            "artifact_requirements": [],
        }],
        "unresolved_issues": [{
            "kind": "inventory_shape_repair",
            "field": "items",
            "message": "exercise the bounded inventory repair",
        }],
    });
    let mut evidence = LifecycleEvidence::default();

    let repaired = driver
        .repair_inventory(inventory, &serde_json::json!([]), &mut evidence)
        .await
        .expect("inventory repair");

    assert_eq!(repaired["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(repaired["items"][0]["item_id"], "scheduled-item");
    assert_eq!(
        repaired["items"][0]["canonical_task_ids"],
        serde_json::json!(["TASK-EX-BOUNDARY"])
    );
    assert!(
        repaired["unresolved_issues"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["inventory-shape-repair-1"]
    );
}

fn boundary_driver(
    temp: &tempfile::TempDir,
    scenario: CannedLifecycleScenario,
) -> (LifecycleDriver, Arc<CannedLifecycleLlm>) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-BOUNDARY".to_string(),
            source_path: "tasks/TASK-EX-BOUNDARY.md".to_string(),
            acceptance_criteria: vec!["Boundary behavior remains semantic.".to_string()],
            ..Default::default()
        }],
    };
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "boundary-preservation-e2e".to_string(),
        task: "Exercise a neutral lifecycle boundary fixture.".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let llm = Arc::new(CannedLifecycleLlm {
        scenario,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        Some(30),
    );
    let generated_config = archon_core::config::GeneratedWorkflowConfig {
        max_repair_iterations: 1,
        max_investigation_iterations: 1,
        verification_branch_timeout_secs: 30,
        host_call_timeout_secs: 30,
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task,
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: generated_config.clone(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id,
        true,
        Some(universe.clone()),
        None,
    );
    let host = Arc::new(WorkflowScriptHost {
        scaffold_hash: workflow_scaffold_hash("# boundary preservation fixture"),
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
    });
    let driver = LifecycleDriver::new(
        host,
        universe,
        None,
        Some(temp.path().display().to_string()),
        serde_json::json!([]),
        Default::default(),
        &generated_config,
    );
    (driver, llm)
}

fn synthetic_task_universe(root: &std::path::Path) -> WorkflowV2TaskUniverse {
    let task = |id: &str, criterion: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: root
            .join("tasks")
            .join(format!("{id}.md"))
            .display()
            .to_string(),
        acceptance_criteria: vec![criterion.to_string()],
        ..Default::default()
    };
    let mut contract_task = task("TASK-EX-004", "Declared artifact verification passes.");
    contract_task.artifact_requirements =
        vec![".archon/artifacts/example-contract.json".to_string()];
    contract_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "example-record".to_string(),
        artifact_path: ".archon/artifacts/example-contract.json".to_string(),
        required_universe: false,
        ..Default::default()
    }];
    let mut artifact_only_task = task("TASK-EX-005", "Artifact-only output is produced.");
    artifact_only_task.artifact_requirements =
        vec![".archon/artifacts/artifact-only.json".to_string()];
    artifact_only_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "artifact-only".to_string(),
        artifact_path: ".archon/artifacts/artifact-only.json".to_string(),
        ..Default::default()
    }];
    let mut parameterized_task = task(
        "TASK-EX-006",
        "Parameterized instance reports are contract-verified.",
    );
    parameterized_task.artifact_requirements =
        vec![".archon/artifacts/instances/<instance-id>/report.json".to_string()];
    parameterized_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "instance_report".to_string(),
        artifact_path: ".archon/artifacts/instances/<instance-id>/report.json".to_string(),
        instance_source_path: Some(".archon/artifacts/instances.json".to_string()),
        instance_source_records_field: Some("records".to_string()),
        instance_artifact_field: Some("report_path".to_string()),
        validation_status_field: Some("status".to_string()),
        validation_checks_field: Some("checks".to_string()),
        validation_check_status_field: Some("status".to_string()),
        validation_failed_values: vec!["failed".to_string()],
        validation_passed_values: vec!["passed".to_string()],
        ..Default::default()
    }];
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec![root.join("tasks").display().to_string()],
        tasks: vec![
            task("TASK-EX-001", "Existing evidence is sufficient."),
            task("TASK-EX-002", "Refuted work is implemented."),
            task("TASK-EX-003", "Plain implementation is present."),
            contract_task,
            artifact_only_task,
            parameterized_task,
        ],
    }
}

fn synthetic_inventory_items() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "item_id": "noop-legit",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-001"],
            "dependency_ids": [],
            "acceptance_criteria": ["Existing evidence is sufficient."],
            "noop_proof": "existing neutral fixture evidence",
            "noop_proof_refs": ["fixture:existing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-refutable",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-002"],
            "dependency_ids": [],
            "acceptance_criteria": ["Refuted work is implemented."],
            "noop_proof": "unsupported inherited claim",
            "noop_proof_refs": ["fixture:missing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-artifact-only",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-005"],
            "dependency_ids": [],
            "acceptance_criteria": ["Artifact-only output is produced."],
            "noop_proof": "unsupported inherited artifact claim",
            "noop_proof_refs": ["fixture:missing-artifact"],
            "artifact_requirements": [],
        }),
        implementation_item(
            "implementation-plain",
            "TASK-EX-003",
            "src/plain.rs",
            "Plain implementation is present.",
        ),
        implementation_item(
            "implementation-contract",
            "TASK-EX-004",
            "src/contract.rs",
            "Declared artifact verification passes.",
        ),
        implementation_item(
            "implementation-parameterized",
            "TASK-EX-006",
            "src/parameterized.rs",
            "Parameterized instance reports are contract-verified.",
        ),
    ]
}

fn implementation_item(
    item_id: &str,
    task_id: &str,
    target_file: &str,
    criterion: &str,
) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "work_type": "implementation",
        "canonical_task_ids": [task_id],
        "dependency_ids": [],
        "target_files": [target_file],
        "acceptance_criteria": [criterion],
        "focused_verification": format!("test -f {target_file}"),
        "artifact_requirements": if task_id == "TASK-EX-004" {
            serde_json::json!([".archon/artifacts/example-contract.json"])
        } else {
            serde_json::json!([])
        },
    })
}

fn verification_item(item_id: &str, task_id: &str, target_file: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "source_item_id": item_id.replace("verify-", "implementation-"),
        "canonical_task_ids": [task_id],
        "focused_verification": format!("test -f {target_file}"),
        "expected_evidence": format!("{target_file} exists"),
        "artifact_requirements": [],
    })
}

fn verification_remediation_item() -> serde_json::Value {
    serde_json::json!({
        "item_id": "remediate-plain",
        "source_item_id": "implementation-plain",
        "work_type": "implementation",
        "canonical_task_ids": ["TASK-EX-003"],
        "dependency_ids": [],
        "target_files": ["src/plain.rs"],
        "failure_status": "needs_review",
        "failure_evidence": "the first focused check failed",
        "required_fix": "re-apply the neutral implementation",
        "focused_verification": "test -f src/plain.rs",
        "artifact_requirements": [],
    })
}

fn noop_proof_result(call_id: &str) -> serde_json::Value {
    if call_id.ends_with("noop-legit") {
        serde_json::json!({
            "status": "noop",
            "summary": "authoritative noop evidence exists",
            "evidence": [{"kind": "inspection", "summary": "fixture evidence checked"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-001",
                "status": "noop",
                "summary": "existing evidence satisfies the criterion",
                "evidence": [{"kind": "inspection", "summary": "fixture:existing-evidence"}],
            }],
            "residual_gaps": [],
            "data": {
                "item_id": "noop-legit",
                "canonical_task_ids": ["TASK-EX-001"],
                "acceptance_criteria_results": [{
                    "task_id": "TASK-EX-001",
                    "criterion": "Existing evidence is sufficient.",
                    "status": "passed",
                    "evidence_refs": ["fixture:existing-evidence"],
                }],
            },
        })
    } else if call_id.contains("artifact-only") {
        serde_json::json!({
            "status": "needs_review",
            "summary": "artifact-only noop claim is refuted",
            "evidence": [{"kind": "inspection", "summary": "required artifact is absent"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-005",
                "status": "missing",
                "summary": "artifact is absent",
                "evidence": [],
            }],
            "residual_gaps": [{
                "id": "gap-refuted-artifact-noop",
                "description": "the declared artifact does not exist",
                "severity": "blocking",
            }],
            "data": {
                "item_id": call_id,
                "canonical_task_ids": ["TASK-EX-005"],
                "proof_gap": true,
            },
        })
    } else {
        serde_json::json!({
            "status": "needs_review",
            "summary": "noop claim is refuted",
            "evidence": [{"kind": "inspection", "summary": "required evidence is absent"}],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [{
                "task_id": "TASK-EX-002",
                "status": "missing",
                "summary": "implementation evidence is absent",
                "evidence": [],
            }],
            "residual_gaps": [{
                "id": "gap-refuted-noop",
                "description": "the exact acceptance criterion is not satisfied",
                "severity": "blocking",
            }],
            "data": {
                "item_id": call_id,
                "canonical_task_ids": ["TASK-EX-002"],
                "proof_gap": true,
            },
        })
    }
}

fn implementation_result(
    request: &AgentExecutionRequest,
    input: &serde_json::Value,
    call_id: &str,
) -> Result<serde_json::Value> {
    let item = find_item(input).ok_or_else(|| anyhow::anyhow!("implementation item missing"))?;
    let task_id = first_string(item.get("canonical_task_ids"))
        .ok_or_else(|| anyhow::anyhow!("canonical task id missing"))?;
    let target_file = first_string(item.get("target_files"));
    let cwd = request
        .cwd
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("cwd missing"))?;
    if let Some(target_file) = target_file.as_deref() {
        let target = cwd.join(target_file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &target,
            format!(
                "pub fn implemented_{}() -> bool {{ true }}\n",
                task_id.replace('-', "_")
            ),
        )?;
    } else if task_id == "TASK-EX-005" {
        let project_root = find_string_key(input, "project_artifact_root")
            .or_else(|| find_string_key(input, "project_root"))
            .ok_or_else(|| anyhow::anyhow!("project artifact root missing"))?;
        std::fs::write(
            std::path::Path::new(&project_root).join(".archon/artifacts/artifact-only.json"),
            "{\"status\":\"produced\"}\n",
        )?;
    } else {
        anyhow::bail!("target file missing")
    }
    let reported_task_id = if task_id == "TASK-EX-003" {
        "EX-003"
    } else {
        task_id.as_str()
    };
    let mut result = accepted_result(
        "implementation branch changed its declared target",
        serde_json::json!({
            "item_id": item
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(call_id),
            "canonical_task_ids": [reported_task_id],
        }),
        vec![coverage(&task_id, "accepted")],
        vec![test_command("true", true, "implementation fixture passed")],
    );
    if let Some(target_file) = target_file {
        result = result.with_files_changed(vec![target_file]);
    }
    if task_id == "TASK-EX-004" {
        let project_root = find_string_key(input, "project_artifact_root")
            .or_else(|| find_string_key(input, "project_root"))
            .ok_or_else(|| anyhow::anyhow!("project artifact root missing"))?;
        result["artifacts"] = serde_json::json!([{
            "id": "example-contract",
            "path": std::path::Path::new(&project_root)
                .join(".archon/artifacts/example-contract.json")
                .display()
                .to_string(),
            "description": "pre-existing declared contract fixture",
        }]);
    }
    Ok(result)
}

fn verification_result(
    request: &AgentExecutionRequest,
    input: &serde_json::Value,
    call_id: &str,
    deliverable_contract_executed: &AtomicBool,
    parameterized_contract_executed: &AtomicBool,
    verification_failure_emitted: &AtomicBool,
) -> Result<serde_json::Value> {
    let item = find_item(input).ok_or_else(|| anyhow::anyhow!("verification item missing"))?;
    let task_id = first_string(item.get("canonical_task_ids"))
        .ok_or_else(|| anyhow::anyhow!("verification task id missing"))?;
    let command = item
        .get("focused_verification")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("true");
    let cwd = request
        .cwd
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("cwd missing"))?;
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()?;
    if item.get("deliverable_contract").is_some() {
        deliverable_contract_executed.store(true, Ordering::SeqCst);
    }
    if item
        .get("deliverable_contract")
        .and_then(|contract| contract.get("kind"))
        .and_then(serde_json::Value::as_str)
        == Some("instance_report")
    {
        parameterized_contract_executed.store(true, Ordering::SeqCst);
    }
    let forced_failure = task_id == "TASK-EX-003"
        && !call_id.contains("post-remediation")
        && !verification_failure_emitted.swap(true, Ordering::SeqCst);
    let succeeded = output.status.success() && !forced_failure;
    let summary = if succeeded {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        if forced_failure {
            "synthetic first-attempt verification failure".to_string()
        } else {
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        }
    };
    let mut result = accepted_result(
        "focused verification executed",
        serde_json::json!({
            "item_id": item
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(call_id),
            "source_item_id": item.get("source_item_id"),
            "canonical_task_ids": [task_id],
            "focused_verification": command,
            "pass_fail_count": {
                "intended_target_passed": usize::from(succeeded),
                "intended_target_failed": usize::from(!succeeded),
            },
            "matched_test_check_names": {
                "passed": if succeeded { vec![call_id] } else { Vec::new() },
                "failed": if succeeded { Vec::new() } else { vec![call_id] },
            },
        }),
        vec![coverage(
            &task_id,
            if succeeded { "accepted" } else { "blocked" },
        )],
        vec![test_command(command, succeeded, &summary)],
    );
    if !succeeded {
        result["status"] = serde_json::json!("needs_review");
        result["residual_gaps"] = serde_json::json!([{
            "id": format!("verification-failed-{call_id}"),
            "description": summary,
            "severity": "blocking",
        }]);
    }
    Ok(result)
}

fn accepted_result(
    summary: &str,
    data: serde_json::Value,
    task_coverage: Vec<serde_json::Value>,
    commands_run: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "accepted",
        "summary": summary,
        "evidence": [{"kind": "inspection", "summary": summary}],
        "artifacts": [],
        "commands_run": commands_run,
        "files_read": [],
        "files_changed": [],
        "task_coverage": task_coverage,
        "residual_gaps": [],
        "data": data,
    })
}

fn needs_review_result(summary: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "status": "needs_review",
        "summary": summary,
        "evidence": [{"kind": "inspection", "summary": summary}],
        "artifacts": [],
        "commands_run": [],
        "files_read": [],
        "files_changed": [],
        "task_coverage": [],
        "residual_gaps": [{
            "id": "synthetic-inventory-shape",
            "description": summary,
            "severity": "blocking",
        }],
        "data": data,
    })
}

trait ResultValueExt {
    fn with_files_changed(self, files: Vec<String>) -> serde_json::Value;
}

impl ResultValueExt for serde_json::Value {
    fn with_files_changed(mut self, files: Vec<String>) -> serde_json::Value {
        self["files_changed"] = serde_json::Value::Array(
            files
                .into_iter()
                .map(|path| serde_json::json!({"path": path, "purpose": "declared target edit"}))
                .collect(),
        );
        self["evidence"] = serde_json::json!([{
            "kind": "implementation",
            "summary": "declared target changed in isolated worktree",
        }]);
        self
    }
}

fn coverage(task_id: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "status": status,
        "summary": format!("{task_id} {status}"),
        "evidence": [{"kind": "test", "summary": format!("{task_id} evidence")}],
    })
}

fn all_task_coverage() -> Vec<serde_json::Value> {
    vec![
        coverage("TASK-EX-001", "noop"),
        coverage("TASK-EX-002", "accepted"),
        coverage("TASK-EX-003", "accepted"),
        coverage("TASK-EX-004", "accepted"),
        coverage("TASK-EX-005", "accepted"),
        coverage("TASK-EX-006", "accepted"),
    ]
}

fn test_command(command: &str, succeeded: bool, summary: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "test",
        "command": command,
        "status": if succeeded { "succeeded" } else { "failed" },
        "exit_code": if succeeded { 0 } else { 1 },
        "output_summary": if summary.is_empty() { "command completed" } else { summary },
    })
}

fn prompt_line(prompt: &str, prefix: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix).map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn prompt_input(prompt: &str) -> serde_json::Value {
    let Some(after) = prompt.split("## Input\n```json\n").nth(1) else {
        return serde_json::Value::Null;
    };
    let Some(raw) = after.split("\n```").next() else {
        return serde_json::Value::Null;
    };
    serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
}

fn find_item(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("canonical_task_ids")
                && (object.contains_key("target_files")
                    || object.contains_key("focused_verification"))
            {
                return Some(value);
            }
            object.values().find_map(find_item)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_item),
        _ => None,
    }
}

fn first_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn find_string_key(value: &serde_json::Value, target: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get(target)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_string_key(value, target))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_string_key(value, target)),
        _ => None,
    }
}

fn init_git_repo(repo: &std::path::Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "archon-test"]);
    run_git(
        repo,
        &["config", "user.email", "archon-test@example.invalid"],
    );
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
