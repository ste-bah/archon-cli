//! Best-effort learning fold for immutable fixed decomposition runs.
use std::path::Path;

use archon_core::config::LearningConfig;
use archon_workflow::{StageStatus, Verification, WorkflowLearningRecord, WorkflowStore};
use serde_json::Value;

pub(crate) fn hooks(content: &str, config: &LearningConfig) -> Vec<String> {
    // This run has no embedding provider for SONA trajectories. DESC has a
    // real consumer (the episode store); ReasoningBank is useful for review
    // and diagnosis, as selected by the shared task classifier.
    crate::command::learning_workflow_hooks::derive_learning_hooks(content, None, config)
        .into_iter()
        .filter(|hook| hook == "reasoning_bank" || hook == "desc")
        .collect()
}

pub(crate) fn stable_lesson_key(run_id: &str, call_id: &str, attempt: u32) -> String {
    format!(
        "decomposition-{}-{}-{}",
        digest(run_id),
        digest(call_id),
        attempt
    )
}

fn digest(text: &str) -> String {
    archon_workflow::workflow_scaffold_hash(text)[..16].to_string()
}

pub(crate) fn redact_lesson(text: &str) -> String {
    bounded(&archon_observability::redaction::redact_text(text))
}

pub(crate) fn lesson_records(
    run_id: &str,
    _content: &str,
    call_id: &str,
    attempt: u32,
    source: &Value,
    hooks: &[String],
) -> Vec<WorkflowLearningRecord> {
    let mut lessons = Vec::new();
    collect_lessons(source, &mut lessons);
    let call_kind = call_id.to_ascii_lowercase();
    if call_kind.contains("author") || call_kind.contains("body") {
        if let Some(text) = find_text(source, &["response", "summary", "text", "content"]) {
            lessons.push((
                "author_attempt".into(),
                format!("Author attempt outcome: {}", bounded(&text)),
                false,
            ));
        }
    }
    if call_kind.contains("probe") || call_kind.contains("baseline") {
        if let Some(text) = find_text(
            source,
            &[
                "baseline_output",
                "baselineOutput",
                "stdout",
                "output",
                "summary",
            ],
        ) {
            lessons.push((
                "probe_baseline".into(),
                format!("Probe baseline output: {}", bounded(&text)),
                false,
            ));
        }
    }
    if call_kind.contains("set-gate") || call_kind.contains("set_gate") {
        if let Some(text) = find_text(source, &["finding", "findings", "reason", "summary"]) {
            lessons.push((
                "set_gate_finding".into(),
                format!("Set gate finding: {}", bounded(&text)),
                true,
            ));
        }
    }
    lessons
        .into_iter()
        .enumerate()
        .filter_map(|(index, (kind, text, failed))| {
            let clean = redact_lesson(&text);
            if clean.trim().is_empty() {
                return None;
            }
            Some(WorkflowLearningRecord {
                run_id: run_id.to_string(),
                name: clean,
                stage_id: format!("{}-{index}", stable_lesson_key(run_id, call_id, attempt)),
                phase: "decomposition_lesson".into(),
                agent: None,
                status: if failed {
                    StageStatus::Failed
                } else {
                    StageStatus::Accepted
                },
                verification: if failed {
                    Verification::Failed
                } else {
                    Verification::Accepted
                },
                durable: false,
                quality_score: None,
                artifact_refs: vec![kind],
                telemetry: archon_workflow::StageTelemetry {
                    attempt,
                    error_class: None,
                    artifact_count: 0,
                },
                trace_ref: None,
                hooks: hooks.to_vec(),
                ts: chrono::Utc::now(),
            })
        })
        .collect()
}

fn find_text(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    if let Some(text) = value.as_str() {
                        return Some(text.to_string());
                    }
                    if value.is_array() || value.is_object() {
                        return Some(value.to_string());
                    }
                }
            }
            map.values().find_map(|child| find_text(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| find_text(child, keys)),
        _ => None,
    }
}

fn bounded(text: &str) -> String {
    text.chars().take(1200).collect()
}

fn collect_lessons(value: &Value, out: &mut Vec<(String, String, bool)>) {
    match value {
        Value::Object(map) => {
            let verdict = map
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let reason = map.get("reason").and_then(Value::as_str).unwrap_or("");
            let counterexample = map
                .get("counterexample")
                .and_then(Value::as_str)
                .unwrap_or("");
            let rule = map
                .get("rule")
                .and_then(Value::as_str)
                .or_else(|| map.get("criterion").and_then(Value::as_str))
                .unwrap_or("");
            if verdict.contains("refut") && (!reason.is_empty() || !counterexample.is_empty()) {
                out.push((
                    "refutation".into(),
                    format!("Refuted check: {reason}; counterexample: {counterexample}"),
                    true,
                ));
            } else if verdict.contains("cannot_pass") || verdict.contains("cannot pass") {
                out.push((
                    "cannot_pass".into(),
                    format!("cannot pass as written; rule: {rule}; reason: {reason}"),
                    true,
                ));
            } else if verdict.contains("accept")
                && (!reason.is_empty() || !counterexample.is_empty())
            {
                out.push((
                    "accepted_correction".into(),
                    format!("Accepted correction: {reason}; counterexample: {counterexample}"),
                    false,
                ));
            }
            for child in map.values() {
                collect_lessons(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_lessons(child, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn filter_new_records(
    records: Vec<WorkflowLearningRecord>,
    known: &mut std::collections::BTreeSet<String>,
) -> Vec<WorkflowLearningRecord> {
    records
        .into_iter()
        .filter(|record| known.insert(record.stage_id.clone()))
        .collect()
}

/// Persist each lesson once, then route the same stream used by generated runs.
pub(crate) fn fold(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    config: &LearningConfig,
) -> anyhow::Result<()> {
    let run = store.load_state(run_id)?;
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let calls = v2.load_call_records()?;
    let classifier_content = calls
        .iter()
        .fold(run.spec.task.clone(), |mut content, call| {
            content.push(' ');
            content.push_str(&call.call.id);
            content.push(' ');
            content.push_str(&call.result.data.to_string());
            content
        });
    let hooks = hooks(&classifier_content, config);
    if hooks.is_empty() {
        return Ok(());
    }
    let journal = store
        .run_dir(run_id)
        .join("learning/decomposition-records.jsonl");
    std::fs::create_dir_all(journal.parent().expect("journal parent"))?;
    let mut known = std::collections::BTreeSet::new();
    if let Ok(text) = std::fs::read_to_string(&journal) {
        for line in text.lines() {
            if let Ok(record) = serde_json::from_str::<WorkflowLearningRecord>(line) {
                known.insert(record.stage_id);
            }
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal)?;
    let candidates: Vec<_> = calls
        .into_iter()
        .filter(|call| {
            !matches!(
                call.status,
                archon_workflow::WorkflowV2Status::Pending
                    | archon_workflow::WorkflowV2Status::Running
            )
        })
        .flat_map(|call| {
            lesson_records(
                run_id,
                &run.spec.task,
                &call.call.id,
                call.attempt,
                &call.result.data,
                &hooks,
            )
        })
        .collect();
    let records = filter_new_records(candidates, &mut known);
    for record in &records {
        use std::io::Write;
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    crate::command::topology_trace::project_workflow_run(cwd, store, run_id);
    crate::command::topology_fold::fold_project_pending_blocking(
        cwd,
        run_id,
        &run.spec.task,
        "default",
    );
    let outcome =
        crate::command::topology_fold::workflow_learning::bridge_workflow_learning_with_records(
            cwd, store, run_id, &records,
        );
    tracing::debug!(%run_id, dispatched = outcome.dispatched, records = records.len(), "fixed decomposition learning fold finished");
    Ok(())
}

pub(crate) fn best_effort_fold<T>(
    run_id: &str,
    fold: impl FnOnce() -> anyhow::Result<T>,
) -> Option<T> {
    match fold() {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, %run_id, "fixed decomposition learning fold failed");
            None
        }
    }
}

pub(crate) async fn fold_best_effort(cwd: &Path, store: &WorkflowStore, run_id: &str) {
    let cwd = cwd.to_path_buf();
    let store = store.clone();
    let run_id = run_id.to_string();
    let log_run_id = run_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let learning = crate::command::workflow_live::load_learning_config_for_fixed(&cwd);
        let _ = best_effort_fold(&run_id, || fold(&cwd, &store, &run_id, &learning));
    })
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, %log_run_id, "fixed decomposition learning worker failed");
    }
}
