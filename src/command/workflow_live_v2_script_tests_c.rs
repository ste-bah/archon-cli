use super::*;

#[test]
fn status_merge_preserves_terminal_or_review_state() {
    assert_eq!(
        merge_v2_status(WorkflowV2Status::Failed, WorkflowV2Status::Accepted),
        WorkflowV2Status::Failed
    );
    assert_eq!(
        merge_v2_status(WorkflowV2Status::NeedsReview, WorkflowV2Status::Accepted),
        WorkflowV2Status::NeedsReview
    );
    assert_eq!(
        merge_v2_status(WorkflowV2Status::Accepted, WorkflowV2Status::Cancelled),
        WorkflowV2Status::Cancelled
    );
}

#[test]
fn read_only_calls_cannot_accept_task_coverage_without_implementation_evidence() {
    let mut result = WorkflowV2Result::accepted("read-only audit claimed completion");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "read task files",
    ));
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Accepted,
        summary: "claimed done".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "inspected requirements",
        )],
    });
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "readonly-audit".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let result = normalize_result_for_call(&execution, result);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.task_coverage[0].status,
        WorkflowV2TaskCoverageStatus::Unknown
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "read_only_task_acceptance_readonly-audit")
    );
}

#[test]
fn artifact_only_focused_verification_can_accept_verification_coverage() {
    let result: WorkflowV2Result =
        serde_json::from_str(archon_test_support::fixtures::WF32_ARTIFACT_VERIFICATION_AGGREGATE)
            .expect("D16 fixture");
    let options = WorkflowV2HostOptions {
        item_kind: Some("focused_verification".to_string()),
        ..WorkflowV2HostOptions::default()
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "verification-wave-fixture".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let result = normalize_result_for_call(&execution, result);

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.task_coverage[0].status,
        WorkflowV2TaskCoverageStatus::Accepted
    );
    assert!(
        !result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("read_only_task_acceptance_"))
    );
}

#[test]
fn empty_items_output_requires_noop_proof() {
    let mut result = WorkflowV2Result::accepted("inventory complete");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "read all tasks",
    ));
    result.data = serde_json::json!({ "items": [] });
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("outputs".to_string(), serde_json::json!(["items"]));
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "implementation-inventory".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options,
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let result = normalize_result_for_call(&execution, result);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "empty_items_output_implementation-inventory")
    );
}

#[test]
fn inline_inventory_output_is_not_persisted_as_an_artifact() {
    let mut result = WorkflowV2Result::accepted("inventory complete");
    result.data = serde_json::json!({ "items": [{ "item_id": "INV-1" }] });
    result.artifacts = vec![
        archon_workflow::WorkflowV2Artifact {
            id: "canonical-inventory".to_string(),
            path: "inline:data.items".to_string(),
            description: None,
        },
        archon_workflow::WorkflowV2Artifact {
            id: "real-report".to_string(),
            path: "artifacts/inventory.json".to_string(),
            description: None,
        },
    ];
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "canonical-implementation-inventory".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let result = normalize_result_for_call(&execution, result);

    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(result.artifacts[0].path, "artifacts/inventory.json");
}

#[tokio::test]
async fn dynamic_loop_host_calls_are_recorded_with_runtime_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "dynamic loop checkpoints".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  let iteration = 1;
  while (iteration <= 2) {
    await w.checkpoint("loop-checkpoint-" + iteration, { iteration });
    iteration += 1;
  }
}
"#,
        )
        .await
        .expect("script summary");

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(summary.executed, 2);
    assert_eq!(summary.completed, 2);
    assert_eq!(
        summary
            .calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        vec!["loop-checkpoint-1", "loop-checkpoint-2"]
    );
    assert!(
        v2_store
            .load_call_record("loop-checkpoint-1")
            .expect("first checkpoint lookup")
            .is_some()
    );
    assert!(
        v2_store
            .load_call_record("loop-checkpoint-2")
            .expect("second checkpoint lookup")
            .is_some()
    );
    let run = workflow_store.load_state(&run.id).expect("run state");
    assert_eq!(
        run.stages
            .get("loop-checkpoint-1")
            .expect("first runtime stage")
            .status,
        StageStatus::Running
    );
    assert_eq!(
        run.stages
            .get("loop-checkpoint-2")
            .expect("second runtime stage")
            .status,
        StageStatus::Running
    );
}

// The scaffold is a native Rust lifecycle with a descriptor record; the
// declared static plan is its single source of truth for approval metadata.
#[test]
fn static_scaffold_plan_has_unique_ids_and_write_isolated_fanouts() {
    use std::collections::BTreeSet;
    let plan =
        super::super::super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();
    assert!(!plan.is_empty());
    let mut seen = BTreeSet::new();
    for call in &plan {
        assert!(seen.insert(call.id.clone()), "duplicate id {}", call.id);
        if call.method == WorkflowV2HostMethod::Fanout {
            assert_eq!(
                call.write_mode,
                Some(WorkflowV2WriteMode::Worktree),
                "{}",
                call.id
            );
        }
    }
}

/// The review diamond, as declared in the approval-time plan.
///
/// `adversarial-review` was a single terminal REDUCE over every task at once.
/// It is now PARALLEL — one reviewer per task — and it is positioned inside the
/// dependency wave, after `verification-wave` and before the wave's completion
/// accounting, because that is where it runs. The terminal reduce survives as
/// `cross-cutting-review`, narrowed to concerns no single-task reviewer can see.
#[test]
fn static_scaffold_plan_declares_the_per_task_review_diamond() {
    let plan =
        super::super::super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();
    let index = |id: &str| {
        plan.iter()
            .position(|call| call.id == id)
            .unwrap_or_else(|| panic!("plan must declare {id}"))
    };
    let adversarial = &plan[index("adversarial-review")];
    assert_eq!(
        adversarial.method,
        WorkflowV2HostMethod::Parallel,
        "per-task review is a map, not a reduce: a reduce has no per-item branch to \
         recover a task id from, which is exactly how attribution was lost"
    );
    assert_eq!(adversarial.write_mode, None, "review is read-only");
    assert_eq!(
        plan[index("cross-cutting-review")].method,
        WorkflowV2HostMethod::Reduce
    );
    assert!(
        plan.iter()
            .all(|call| call.id != "adversarial-review-reduce"),
        "the old terminal adversarial reduce must not survive under any name"
    );
    assert!(
        index("verification-wave") < index("adversarial-review"),
        "a task is reviewed after its own verification"
    );
    assert!(
        index("adversarial-review") < index("wave-completion-evidence-repair"),
        "per-task review runs inside the wave, not after every wave"
    );
    assert!(
        index("adversarial-review") < index("cross-cutting-review"),
        "the cross-task reduce consumes the per-task findings"
    );
    // Downstream review stages are untouched and still run after the reduce.
    for downstream in [
        "review-remediation-inventory",
        "review-remediation-wave",
        "review-verification-plan",
        "review-verification-wave",
        "blocked-review-unresolved",
        "final-acceptance-report",
    ] {
        assert!(
            index("cross-cutting-review") < index(downstream),
            "{downstream} must still follow the terminal review reduce"
        );
    }
}

#[tokio::test]
async fn dry_run_rejects_provider_routing_and_duplicate_ids() {
    let duplicate = r#"
async function workflow(w) {
  await w.checkpoint("same-id");
  await w.checkpoint("same-id");
}
"#;
    let error = dry_run_workflow_plan(duplicate, None)
        .await
        .expect_err("duplicate ids must fail");
    assert!(
        error.to_string().contains("duplicate host call id"),
        "{error}"
    );

    let routed = r#"
async function workflow(w) {
  await w.agent("routed", { model: "claude-opus-4-8", task: "inspect" });
}
"#;
    let error = dry_run_workflow_plan(routed, None)
        .await
        .expect_err("model override must fail");
    assert!(
        error
            .to_string()
            .contains("provider routing is host policy"),
        "{error}"
    );

    let syntax_error = r#"
async function workflow(w) {
  await w.agent("broken", { task: "inspect" }
}
"#;
    let error = dry_run_workflow_plan(syntax_error, None)
        .await
        .expect_err("syntax error must fail");
    assert!(error.to_string().contains("validation failed"), "{error}");
}

#[tokio::test]
async fn determinism_prelude_blocks_wall_clock_and_randomness() {
    for forbidden in ["Math.random()", "Date.now()", "new Date()"] {
        let source = format!(
            r#"
async function workflow(w) {{
  const value = {forbidden};
  await w.checkpoint("uses-nondeterminism-" + value);
}}
"#
        );
        let error = dry_run_workflow_plan(&source, None)
            .await
            .expect_err("nondeterministic script must fail");
        assert!(
            error
                .to_string()
                .contains("unavailable in workflow scripts"),
            "{forbidden}: {error}"
        );
    }
}

pub(super) fn failed_record_with_summary(call_id: &str, summary: &str) -> WorkflowV2CallRecord {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: summary.to_string(),
        ..WorkflowV2Result::default()
    };
    WorkflowV2CallRecord::new(
        "run",
        WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        String::new(),
        result,
        Vec::new(),
    )
}

#[test]
fn infra_transport_stage_failure_contributes_blocked_not_failed() {
    // A mandatory review that fails purely on a transport/compaction
    // (infrastructure) error must NOT doom an otherwise-complete run to Failed
    // and discard every honest task outcome. It contributes Blocked instead —
    // honest and resumable.
    let record = failed_record_with_summary(
        "adversarial-review-47",
        "workflow stage failed: agent transport failed: reactive subagent compaction failed: no safe compaction boundary",
    );
    assert_eq!(
        run_terminal_status_contribution(&record, WorkflowV2Status::Failed),
        WorkflowV2Status::Blocked,
    );
    // A run whose worst genuine outcome is Blocked (some tasks honestly blocked)
    // stays Blocked when the review infra-fails — not escalated to Failed.
    assert_eq!(
        merge_v2_status(
            WorkflowV2Status::Blocked,
            run_terminal_status_contribution(&record, WorkflowV2Status::Failed),
        ),
        WorkflowV2Status::Blocked,
    );
}

#[test]
fn genuine_stage_failure_still_contributes_failed() {
    // A real work failure (not infrastructure) is unchanged: it still dooms the
    // run to Failed. The downgrade is strictly for transport/compaction errors.
    let record = failed_record_with_summary(
        "implement-task-tdl-010-3",
        "verification found the acceptance criteria unmet",
    );
    assert_eq!(
        run_terminal_status_contribution(&record, WorkflowV2Status::Failed),
        WorkflowV2Status::Failed,
    );
}
