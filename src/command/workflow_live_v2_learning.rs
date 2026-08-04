use super::*;

pub(super) fn record_generated_learning_event(
    store: &WorkflowStore,
    run_id: &str,
    plan: &WorkflowScriptPlan,
    summary: &workflow_live_v2_script::WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<PathBuf> {
    let scaffold_hash = plan.scaffold_hash();
    let evidence_refs = generated_learning_evidence_refs(store, run_id, summary, v2_store);
    let event = WorkflowLearningEvent::generated_run(
        run_id,
        scaffold_hash,
        summary.status,
        generated_failure_class(summary),
        generated_prevented_false_completion(summary),
        evidence_refs,
    );
    let event = attach_generated_learning_runtime_summary(event, summary, v2_store)?;
    let rel = PathBuf::from("learning").join("generated-workflow-events.jsonl");
    let path = store.run_dir(run_id).join(&rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::Io {
            path: parent.to_path_buf(),
            source: err,
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| WorkflowError::Io {
            path: path.clone(),
            source: err,
        })?;
    writeln!(file, "{}", serde_json::to_string(&event)?).map_err(|err| WorkflowError::Io {
        path: path.clone(),
        source: err,
    })?;
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        WorkflowEventKind::LearningRecorded,
        serde_json::json!({
            "event": "generated_workflow_learning_recorded",
            "scaffold_hash": event.scaffold_hash,
            "terminal_status": event.terminal_status,
            "failure_class": event.failure_class,
            "prevented_false_completion": event.prevented_false_completion,
            "call_status_counts": event.call_status_counts,
            "branch_status_counts": event.branch_status_counts,
            "failure_class_counts": event.failure_class_counts,
            "repair_decisions": event.repair_decisions,
            "evidence_gap_refs": event.evidence_gap_refs,
            "canary_result": event.canary_result,
            "path": rel.display().to_string(),
        }),
    )?;
    Ok(path)
}

fn generated_learning_evidence_refs(
    store: &WorkflowStore,
    run_id: &str,
    summary: &workflow_live_v2_script::WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
) -> Vec<WorkflowLearningEvidenceRef> {
    let mut refs = vec![
        WorkflowLearningEvidenceRef::path(
            "workflow_js",
            store
                .run_dir(run_id)
                .join("workflow.js")
                .display()
                .to_string(),
        ),
        WorkflowLearningEvidenceRef::path("v2_results", v2_store.root().display().to_string()),
        WorkflowLearningEvidenceRef::path(
            "generated_metadata",
            store
                .run_dir(run_id)
                .join(GENERATED_V2_METADATA_PATH)
                .display()
                .to_string(),
        ),
    ];
    if let Some(call_id) = summary.failed_call.as_ref() {
        refs.push(WorkflowLearningEvidenceRef::call("failed_call", call_id));
    }
    if let Some(path) = summary.failed_result_path.as_ref() {
        refs.push(WorkflowLearningEvidenceRef::path("failed_result", path));
    }
    refs
}

fn generated_failure_class(
    summary: &workflow_live_v2_script::WorkflowV2ScriptSummary,
) -> Option<String> {
    match summary.status {
        WorkflowV2Status::Failed => Some("failed".to_string()),
        WorkflowV2Status::Blocked => Some("blocked".to_string()),
        WorkflowV2Status::NeedsReview => Some("needs_review".to_string()),
        WorkflowV2Status::Cancelled => Some("cancelled".to_string()),
        _ => None,
    }
}

fn generated_prevented_false_completion(
    summary: &workflow_live_v2_script::WorkflowV2ScriptSummary,
) -> bool {
    !matches!(
        summary.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && summary.calls.iter().any(|call| {
        call.id == "blocked-final-readiness"
            || call.id == "final-acceptance-gate"
            || call.id == "final-zero-gap-audit"
            || call.id.starts_with("blocked-final-")
    })
}

fn attach_generated_learning_runtime_summary(
    event: WorkflowLearningEvent,
    summary: &workflow_live_v2_script::WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowLearningEvent> {
    let records = v2_store.load_call_records()?;
    let mut call_status_counts = BTreeMap::new();
    let mut branch_status_counts = BTreeMap::new();
    let mut failure_class_counts = BTreeMap::new();
    let mut repair_decisions = Vec::new();
    let mut evidence_gap_refs = Vec::new();

    for record in &records {
        increment_count(
            &mut call_status_counts,
            workflow_v2_status_label(record.status),
        );
        if let Some(class) = generated_record_failure_class(record) {
            increment_count(&mut failure_class_counts, class);
        }
        if is_repair_decision_call(&record.call.id) {
            repair_decisions.push(format!(
                "{}:{}",
                record.call.id,
                workflow_v2_status_label(record.status)
            ));
        }
        for gap in &record.result.residual_gaps {
            evidence_gap_refs.push(format!("{}:{}", record.call.id, gap.id));
        }
        if let Some(outcomes) = record
            .result
            .data
            .get("outcomes")
            .and_then(serde_json::Value::as_array)
        {
            for outcome in outcomes {
                let status = outcome
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|status| !status.is_empty())
                    .unwrap_or("unknown");
                increment_count(&mut branch_status_counts, status);
                if let Some(kind) = outcome
                    .get("failure_kind")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|kind| !kind.is_empty())
                {
                    increment_count(&mut failure_class_counts, kind);
                }
            }
        }
    }

    repair_decisions.sort();
    repair_decisions.dedup();
    evidence_gap_refs.sort();
    evidence_gap_refs.dedup();

    Ok(event.with_runtime_summary(
        call_status_counts,
        branch_status_counts,
        failure_class_counts,
        repair_decisions,
        evidence_gap_refs,
        Some(workflow_v2_status_label(summary.status).to_string()),
    ))
}

fn increment_count(counts: &mut BTreeMap<String, usize>, key: impl Into<String>) {
    *counts.entry(key.into()).or_insert(0) += 1;
}

fn generated_record_failure_class(record: &WorkflowV2CallRecord) -> Option<&'static str> {
    match record.status {
        WorkflowV2Status::Failed => Some("failed"),
        WorkflowV2Status::Blocked => Some("blocked"),
        WorkflowV2Status::NeedsReview => Some("needs_review"),
        WorkflowV2Status::Cancelled => Some("cancelled"),
        _ => None,
    }
}

fn is_repair_decision_call(call_id: &str) -> bool {
    call_id.starts_with("remediation-inventory-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-inventory-")
        || call_id.starts_with("review-remediation-wave-")
        || call_id.starts_with("blocked-")
}

fn workflow_v2_status_label(status: WorkflowV2Status) -> &'static str {
    match status {
        WorkflowV2Status::Pending => "pending",
        WorkflowV2Status::Running => "running",
        WorkflowV2Status::Accepted => "accepted",
        WorkflowV2Status::Noop => "noop",
        WorkflowV2Status::Failed => "failed",
        WorkflowV2Status::Blocked => "blocked",
        WorkflowV2Status::NeedsReview => "needs_review",
        WorkflowV2Status::Cancelled => "cancelled",
    }
}
