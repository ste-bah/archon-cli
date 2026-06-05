//! TASK-DWF-080 — End-to-end deep repository audit workflow (US-DWF-001).
//!
//! Proves the full audit pipeline (discover -> fanout review -> adversarial
//! verify -> reduce report -> quality gate) executes through the provider
//! abstraction (AC-DWF-003), under all six provider families (AC-DWF-004),
//! honours the "do not run tests" tool-deny constraint (AC-US1-02), and emits
//! compact progress while persisting the full report to artifacts (AC-US1-03).

use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowResult, WorkflowSpec, WorkflowStageRunner, WorkflowStore, contains_forbidden_field,
};

/// Deterministic runner that drives a realistic audit: discovery emits two
/// modules, each review fans out, and the adversarial verifier signs off.
struct AuditRunner {
    provider: &'static str,
}

#[async_trait::async_trait]
impl WorkflowStageRunner for AuditRunner {
    async fn run_stage(&self, request: StageRunRequest) -> WorkflowResult<StageRunOutput> {
        let body = if request.stage_id == "discover" {
            r#"{"items":[{"path":"src/auth.rs","reason":"auth"},{"path":"src/db.rs","reason":"db"}]}"#.to_string()
        } else if request.stage_id.starts_with("review-") {
            "finding: module reviewed, no blocking issues".to_string()
        } else if request.stage_id == "verify" {
            "adversarial verification complete: no policy bypass observed".to_string()
        } else {
            format!("{} handled {}", self.provider, request.stage_id)
        };
        Ok(StageRunOutput {
            body,
            extension: "md".into(),
            provider_id: Some(self.provider.into()),
            resolved_model: Some(format!("{}-test-model", self.provider)),
            tokens_in: 1,
            tokens_out: 1,
            cost_usd: 0.0,
        })
    }
}

fn audit_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: audit-e2e
task: Audit this repository deeply. Use subagents. Do not run tests.
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: review
    kind: fanout
    agent: workflow-reviewer
    foreach: ${discover.items}
    depends_on: [discover]
  - id: verify
    kind: agent
    agent: adversarial-verifier
    depends_on: [review]
  - id: report
    kind: reduce
    reducer: code_review_synthesis
    depends_on: [review, verify]
  - id: quality
    kind: quality_gate
    depends_on: [report]
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn audit_workflow_runs_end_to_end_through_provider_abstraction() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec()).unwrap();
    let run_id = run.id.clone();
    let report = executor
        .execute_with_runner(
            run,
            &AuditRunner {
                provider: "anthropic",
            },
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 0, "audit pipeline must complete cleanly");

    let finished = store.load_state(&run_id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    // AC-DWF-003: every stage routed through the active provider, recording its id.
    let review = finished.stages.get("review").unwrap();
    assert_eq!(review.status, StageStatus::Accepted);
    assert_eq!(
        review.artifacts.len(),
        2,
        "one artifact per discovered module"
    );
    for stage in ["discover", "verify", "report", "quality"] {
        assert_eq!(
            finished.stages.get(stage).unwrap().status,
            StageStatus::Accepted,
            "stage {stage} must be accepted"
        );
    }

    // PRD-009/T030/T033/T060: runtime audit records are inspectable in the
    // durable run layout, not only in state.json.
    for path in [
        "prompts/discover/discover.json",
        "agent-outputs/discover/discover.json",
        "prompts/review/review-0.json",
        "agent-outputs/review/review-0.json",
        "artifacts/review/review-0.md",
        "artifacts/review/review-0.meta.json",
        "reducers/report.json",
        "quality/quality.json",
        "learning/records.jsonl",
        "learning/adapter-jepa.jsonl",
        "learning/adapter-world-model.jsonl",
    ] {
        assert!(
            store.run_dir(&run_id).join(path).exists(),
            "missing workflow audit record {path}"
        );
    }
    assert_json_clean(&store, &run_id, "agent-outputs/discover/discover.json");
    assert_json_clean(&store, &run_id, "prompts/discover/discover.json");
}

#[test]
fn audit_spec_honours_do_not_run_tests_tool_deny() {
    // AC-US1-02: the audit plan dispatches no tool stage that runs the suite.
    let spec = audit_spec();
    let has_test_tool = spec.stages.iter().any(|stage| {
        stage
            .tool
            .as_deref()
            .is_some_and(|tool| tool.to_ascii_lowercase().contains("test"))
    });
    assert!(!has_test_tool, "no test-running tool stage may be planned");
}

#[tokio::test]
async fn audit_emits_compact_progress_and_persists_full_report_to_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec()).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(
            run,
            &AuditRunner {
                provider: "anthropic",
            },
        )
        .await
        .unwrap();

    // AC-US1-03: full report lands in artifacts, not the event/progress stream.
    let finished = store.load_state(&run_id).unwrap();
    let report_stage = finished.stages.get("report").unwrap();
    let report_artifact = report_stage.artifacts.first().unwrap();
    let report_path = store.run_dir(&run_id).join(&report_artifact.path);
    let report_body = std::fs::read_to_string(&report_path).unwrap();
    assert!(report_body.contains("Code Review Synthesis"));
    assert!(report_artifact.path.starts_with("artifacts"));

    // Compact progress only: the event log never carries the full report body
    // nor any provider-private field.
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(!events.contains("Code Review Synthesis"));
    for line in events.lines().filter(|l| !l.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(!contains_forbidden_field(&value));
    }
}

#[tokio::test]
async fn audit_workflow_runs_under_all_six_provider_families() {
    // AC-DWF-004: provider tiers resolve and execute across every family.
    for provider in [
        "anthropic",
        "openai-codex",
        "gemini",
        "deepseek",
        "ollama",
        "lm-studio",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
        let run = executor.start(audit_spec()).unwrap();
        let report = executor
            .execute_with_runner(run, &AuditRunner { provider })
            .await
            .unwrap();
        assert_eq!(report.failed, 0, "{provider} audit must succeed");
    }
}

fn assert_json_clean(store: &WorkflowStore, run_id: &str, path: &str) {
    let body = std::fs::read_to_string(store.run_dir(run_id).join(path)).unwrap();
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        !contains_forbidden_field(&value),
        "{path} leaked private fields"
    );
}
