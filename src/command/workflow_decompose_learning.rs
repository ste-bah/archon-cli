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

pub(super) fn safe_fold_context(task: &str) -> String {
    redact_lesson(task)
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
        Value::String(text) if text.to_ascii_lowercase().contains("cannot pass as written") => {
            out.push(("cannot_pass".into(), text.clone(), true));
        }
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
    with_fold_lock(store, run_id, |locked| {
        fold_locked(cwd, locked, run_id, config).map_err(|error| {
            archon_workflow::WorkflowError::SpecInvalid(format!(
                "decomposition learning fold: {error:#}"
            ))
        })
    })?;
    Ok(())
}

pub(super) fn with_fold_lock<T>(
    store: &WorkflowStore,
    run_id: &str,
    operation: impl FnOnce(&WorkflowStore) -> archon_workflow::WorkflowResult<T>,
) -> archon_workflow::WorkflowResult<T> {
    store.with_run_lock(run_id, operation)
}

fn fold_locked(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    config: &LearningConfig,
) -> anyhow::Result<()> {
    let run = store.load_state(run_id)?;
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let calls = v2.load_call_records()?;
    let safe_task = safe_fold_context(&run.spec.task);
    let classifier_content = calls.iter().fold(safe_task.clone(), |mut content, call| {
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
    let pending_journal = store
        .run_dir(run_id)
        .join("learning/decomposition-pending.jsonl");
    std::fs::create_dir_all(journal.parent().expect("journal parent"))?;
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
                &safe_task,
                &call.call.id,
                call.attempt,
                &call.result.data,
                &hooks,
            )
        })
        .collect();
    let records = stage_journal(&journal, &pending_journal, candidates)?;
    crate::command::topology_trace::project_workflow_run(cwd, store, run_id);
    crate::command::topology_fold::fold_project_pending_blocking(
        cwd, run_id, &safe_task, "default",
    );
    let outcome =
        crate::command::topology_fold::workflow_learning::bridge_workflow_learning_with_records(
            cwd, store, run_id, &records,
        );
    finish_journal_dispatch(
        &pending_journal,
        !outcome.integration_unavailable && outcome.dispatched >= records.len(),
    )?;
    tracing::debug!(%run_id, dispatched = outcome.dispatched, records = records.len(), "fixed decomposition learning fold finished");
    Ok(())
}

pub(super) fn read_journal(path: &Path) -> anyhow::Result<Vec<WorkflowLearningRecord>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    text.lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn stage_journal(
    history_path: &Path,
    pending_path: &Path,
    candidates: Vec<WorkflowLearningRecord>,
) -> anyhow::Result<Vec<WorkflowLearningRecord>> {
    let mut history = read_journal(history_path)?;
    let mut known = history
        .iter()
        .map(|record| record.stage_id.clone())
        .collect();
    let new_records = filter_new_records(candidates, &mut known);
    let pending = merge_records(read_journal(pending_path)?, new_records.clone());
    // The retry source is durable before the seen journal advances. A crash
    // between the writes can therefore replay a pending record, never lose it.
    write_journal(pending_path, &pending)?;
    history.extend(new_records);
    write_journal(history_path, &history)?;
    Ok(pending)
}

fn merge_records(
    mut records: Vec<WorkflowLearningRecord>,
    candidates: Vec<WorkflowLearningRecord>,
) -> Vec<WorkflowLearningRecord> {
    let mut known = records
        .iter()
        .map(|record| record.stage_id.clone())
        .collect();
    records.extend(filter_new_records(candidates, &mut known));
    records
}

pub(super) fn write_journal(path: &Path, records: &[WorkflowLearningRecord]) -> anyhow::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    for record in records {
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub(super) fn finish_journal_dispatch(path: &Path, dispatched: bool) -> anyhow::Result<()> {
    if dispatched {
        write_journal(path, &[])?;
    }
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
