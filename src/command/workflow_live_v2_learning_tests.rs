use super::*;
use archon_workflow::{WorkflowV2AgentRequest, WorkflowV2HostOptions};

struct LearningFixture {
    _temp: tempfile::TempDir,
    store_root: PathBuf,
    run_id: String,
    plan: WorkflowScriptPlan,
    v2_root: PathBuf,
    implementation: WorkflowV2CallRecord,
    verification: WorkflowV2CallRecord,
    implementation_marker: String,
    verification_marker: String,
}

fn learning_fidelity_call(id: &str, method: WorkflowV2HostMethod) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    }
}

fn learning_fidelity_record(
    run_id: &str,
    call: WorkflowV2HostCall,
    status: WorkflowV2Status,
    marker: &str,
    outcome_status: &str,
    failure_kind: Option<&str>,
) -> WorkflowV2CallRecord {
    let mut outcome = serde_json::json!({
        "item_id": format!("{}-item", call.id),
        "canonical_task_ids": ["TASK-1"],
        "status": outcome_status,
        "summary": "concise branch outcome",
        "full_detail": marker,
    });
    if let Some(kind) = failure_kind {
        outcome["failure_kind"] = serde_json::json!(kind);
    }
    let result = WorkflowV2Result {
        status,
        summary: "concise call result".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            marker,
        )],
        data: serde_json::json!({"outcomes":[outcome]}),
        residual_gaps: residual_gaps(status, marker),
        ..WorkflowV2Result::default()
    };
    WorkflowV2CallRecord::new(
        run_id,
        call,
        0,
        format!("input-{marker}"),
        result,
        Vec::new(),
    )
}

fn residual_gaps(status: WorkflowV2Status, marker: &str) -> Vec<WorkflowV2ResidualGap> {
    (status == WorkflowV2Status::NeedsReview)
        .then(|| WorkflowV2ResidualGap {
            id: "verification-gap".to_string(),
            description: marker.to_string(),
            severity: Some("high".to_string()),
        })
        .into_iter()
        .collect()
}

fn create_learning_fixture() -> LearningFixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join(".archon/workflows");
    let store = WorkflowStore::new(&store_root);
    let plan = WorkflowScriptPlan::generated(
        "learning fidelity",
        "async function workflow(w) {}",
        Vec::new(),
        None,
        GeneratedWorkflowConfig::default(),
    );
    let run = store
        .create_run(plan.approval_metadata_spec())
        .expect("create workflow run");
    let v2_root = store.run_dir(&run.id).join("v2");
    let implementation_marker = format!("full-implementation-proof-{}", "i".repeat(4096));
    let verification_marker = format!("full-verification-proof-{}", "v".repeat(4096));
    let implementation = learning_fidelity_record(
        &run.id,
        learning_fidelity_call("remediation-wave-1", WorkflowV2HostMethod::Reduce),
        WorkflowV2Status::Accepted,
        &implementation_marker,
        "accepted",
        None,
    );
    let verification = learning_fidelity_record(
        &run.id,
        learning_fidelity_call("verification-wave-1", WorkflowV2HostMethod::Reduce),
        WorkflowV2Status::NeedsReview,
        &verification_marker,
        "needs_review",
        Some("verification_failed"),
    );
    LearningFixture {
        _temp: temp,
        store_root,
        run_id: run.id,
        plan,
        v2_root,
        implementation,
        verification,
        implementation_marker,
        verification_marker,
    }
}

fn save_full_records(fixture: &LearningFixture) {
    let store = WorkflowV2ResultStore::new(&fixture.v2_root);
    store
        .save_call_record(&fixture.implementation)
        .expect("save implementation record");
    store
        .save_call_record(&fixture.verification)
        .expect("save verification record");
}

fn assert_prompt_projection(fixture: &LearningFixture) {
    let input = serde_json::json!({
        "implementationEvidence": [
            {"kind":"implementation-wave","implementationWaveIndex":1,"result":fixture.implementation.result},
            {"kind":"verification-wave","verificationRepairAttempt":1,"result":fixture.verification.result}
        ]
    });
    let mut request = WorkflowV2AgentRequest {
        call: learning_fidelity_call("full-evidence-agent", WorkflowV2HostMethod::Agent),
        role: "researcher".to_string(),
        task: "inspect evidence".to_string(),
        constraints: Vec::new(),
        input,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    };
    let adapter = WorkflowV2AgentAdapter::new();
    let full = adapter.build_prompt_parts(&request).invocation;
    assert!(full.contains(&fixture.implementation_marker));
    request.call = learning_fidelity_call("summary-reducer", WorkflowV2HostMethod::Reduce);
    let digested = adapter.build_prompt_parts(&request).invocation;
    assert!(!digested.contains(&fixture.implementation_marker));
    assert!(digested.contains(&fixture.verification_marker));
}

fn assert_full_records(reloaded: &[WorkflowV2CallRecord], fixture: &LearningFixture) {
    assert_eq!(
        reloaded,
        [fixture.implementation.clone(), fixture.verification.clone()]
    );
    let serialized = serde_json::to_string(reloaded).expect("serialize reloaded records");
    assert!(serialized.contains(&fixture.implementation_marker));
    assert!(serialized.contains(&fixture.verification_marker));
    assert!(serialized.contains("verification_failed"));
}

fn record_learning_event(fixture: &LearningFixture) -> PathBuf {
    let store = WorkflowStore::new(&fixture.store_root);
    let v2_store = WorkflowV2ResultStore::new(&fixture.v2_root);
    assert_full_records(
        &v2_store.load_call_records().expect("reload full records"),
        fixture,
    );
    let summary = workflow_live_v2_script::WorkflowV2ScriptSummary {
        status: WorkflowV2Status::NeedsReview,
        completed: 2,
        executed: 2,
        reused: 0,
        calls: vec![
            fixture.implementation.call.clone(),
            fixture.verification.call.clone(),
        ],
        failed_call: None,
        failed_result_path: None,
        script_result: None,
        next_action: None,
    };
    record_generated_learning_event(&store, &fixture.run_id, &fixture.plan, &summary, &v2_store)
        .expect("record learning event")
}

fn assert_learning_event(path: &Path, fixture: &LearningFixture) {
    let line = std::fs::read_to_string(path).expect("read learning event artifact");
    let event: serde_json::Value = serde_json::from_str(line.trim()).expect("event JSON");
    assert_eq!(event["call_status_counts"]["accepted"], 1);
    assert_eq!(event["call_status_counts"]["needs_review"], 1);
    assert_eq!(event["branch_status_counts"]["accepted"], 1);
    assert_eq!(event["branch_status_counts"]["needs_review"], 1);
    assert_eq!(event["failure_class_counts"]["needs_review"], 1);
    assert_eq!(event["failure_class_counts"]["verification_failed"], 1);
    assert_eq!(
        event["repair_decisions"],
        serde_json::json!(["remediation-wave-1:accepted"])
    );
    assert_eq!(
        event["evidence_gap_refs"],
        serde_json::json!(["verification-wave-1:verification-gap"])
    );
    let store = WorkflowV2ResultStore::new(&fixture.v2_root);
    assert_full_records(&store.load_call_records().expect("final reload"), fixture);
}

#[test]
fn generated_learning_store_fidelity_survives_prompt_digest_and_reopen() {
    let fixture = create_learning_fixture();
    save_full_records(&fixture);
    assert_prompt_projection(&fixture);
    let event_path = record_learning_event(&fixture);
    assert_learning_event(&event_path, &fixture);
}
