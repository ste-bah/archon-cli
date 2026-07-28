use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_tui::event_channel::bounded_tui_event_channel;
use archon_workflow::{
    WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::*;
use crate::command::workflow_live::workflow_live_task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

struct CannedLifecycleLlm {
    calls: Mutex<Vec<String>>,
    deliverable_contract_executed: AtomicBool,
    inventory_calls: AtomicUsize,
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

        let content = if call_id == "canonical-implementation-inventory" {
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
        {
            implementation_result(&request, &input, &call_id)?
        } else if request.task.contains("Run focused verification only") {
            verification_result(
                &request,
                &input,
                &call_id,
                &self.deliverable_contract_executed,
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
async fn real_decomposed_lifecycle_demotes_discovers_implements_and_reaches_terminal() {
    let started = Instant::now();
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    for path in ["src/refuted.rs", "src/plain.rs", "src/contract.rs"] {
        std::fs::write(repo.join(path), "// pending\n").expect("seed source");
    }
    init_git_repo(&repo);
    std::fs::create_dir_all(temp.path().join(".archon/artifacts")).expect("artifact root");
    std::fs::write(
        temp.path().join(".archon/artifacts/example-contract.json"),
        r#"{"status":"ready","records":[{"id":"example","value":1}]}"#,
    )
    .expect("stub artifact");

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
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
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
    assert_eq!(report["accepted_tasks"].as_array().map(Vec::len), Some(4));
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
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
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

include!("workflow_live_v2_lifecycle_e2e_fixtures.rs");
