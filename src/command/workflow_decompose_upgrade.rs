//! Upgrade admission without mutating the immutable launch snapshot.
use super::*;
use crate::command::workflow_decompose_transitions::{
    self as transitions, RuntimeTransition, RuntimeTransitions,
};

pub(crate) fn unmapped(field: &str, reason: &str) -> anyhow::Error {
    anyhow!(
        "fixed decomposition resume paused: cannot map {field}: {reason}; restore an intact record or install a compatible binary with an explicit migration, then resume this run"
    )
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    record: &str,
) -> Result<T> {
    serde_path_to_error::deserialize(value).map_err(|error| {
        let path = error.path().to_string();
        let mut field = if path == "." {
            record.to_string()
        } else {
            format!("{record}.{path}")
        };
        let reason = error.inner().to_string();
        if let Some(missing) = reason
            .strip_prefix("missing field `")
            .or_else(|| reason.strip_prefix("unknown field `"))
            .and_then(|s| s.split('`').next())
        {
            field.push('.');
            field.push_str(missing);
        }
        unmapped(&field, &reason)
    })
}

/// No result-store healing, ignored archive gaps, or unknown future schemas
/// may turn unreadable execution history into a fresh call on upgrade.
pub(super) fn validate_result_state(store: &WorkflowStore, run_id: &str) -> Result<()> {
    let root = store.run_dir(run_id).join("v2");
    let checkpoint = root.join("checkpoint.json");
    if checkpoint.exists() {
        let value = serde_json::from_slice(&std::fs::read(&checkpoint)?)
            .map_err(|e| unmapped("v2/checkpoint.json", &e.to_string()))?;
        let _: archon_workflow::WorkflowV2Checkpoint = decode(value, "v2/checkpoint.json")?;
    }
    validate_call_directory(
        &root.join("results"),
        &archon_workflow::WorkflowV2ResultStore::new(&root),
        true,
    )
}

fn validate_call_directory(
    path: &Path,
    store: &archon_workflow::WorkflowV2ResultStore,
    slots: bool,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let file = entry.path();
        if entry.file_type()?.is_dir() {
            if entry.file_name() != "quarantine" {
                validate_call_directory(&file, store, false)?;
            }
        } else if file
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let field = file.display().to_string();
            let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)
                .map_err(|e| unmapped(&field, &e.to_string()))?;
            // Missing schema is the explicitly supported pre-versioned v2
            // shape (the result store's existing default). Unknown is not.
            if value.get("schema_version").is_some()
                && value["schema_version"] != "workflow-result-v2"
            {
                return Err(unmapped(
                    &format!("{field}.schema_version"),
                    "unsupported call record schema",
                ));
            }
            if let Some(graph) = value.get("source_task_graph").filter(|v| !v.is_null())
                && graph["schema_version"] != "workflow-v2-source-task-graph-v1"
            {
                return Err(unmapped(
                    &format!("{field}.source_task_graph.schema_version"),
                    "unsupported source graph schema",
                ));
            }
            let record: archon_workflow::WorkflowV2CallRecord = decode(value, &field)?;
            if !record.run_id.is_empty() && record.run_id != store.run_id() {
                return Err(unmapped(
                    &format!("{field}.run_id"),
                    "belongs to a different workflow run",
                ));
            }
            if record.call.method == archon_workflow::WorkflowV2HostMethod::HostCommand
                && matches!(
                    record.status,
                    archon_workflow::WorkflowV2Status::Accepted
                        | archon_workflow::WorkflowV2Status::Noop
                        | archon_workflow::WorkflowV2Status::NeedsReview
                )
            {
                let _: archon_workflow::HostCommandResult =
                    decode(record.result.data.clone(), &format!("{field}.result.data"))?;
            }
            if slots && store.result_path(&record.call.id) != file {
                return Err(unmapped(
                    &format!("{field}.call.id"),
                    "does not name this result-store slot",
                ));
            }
        }
    }
    Ok(())
}

/// One durable transition per runtime change, even if preparation is
/// retried or a crash cuts it short. The last transition is read from its own
/// record (`workflow_decompose_transitions`), never from the event log, so a
/// rollback is a transition too and a torn event line blocks nothing. A run
/// without the record reads its launch identity as the last runtime. Launch
/// identity and bundle are never replaced. Returns the transition this call
/// recorded, if any.
pub(super) fn record_upgrade(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    launch: &FixedRunIdentityV1,
    current: &FixedRunIdentityV1,
) -> Result<Option<RuntimeTransition>> {
    let mut record = read_transitions(store, run_id)?.unwrap_or_default();
    let previous = record
        .transitions
        .last()
        .map_or_else(|| launch.clone(), |last| last.new.clone());
    let changed = !transitions::same_runtime(&previous, current);
    if changed {
        record
            .transitions
            .push(RuntimeTransition::new(previous, current.clone()));
        store.write_run_json(run_id, transitions::TRANSITIONS_PATH, &record)?;
    }
    // The visible copies of every transition, each written once: normally
    // only the last one lacks any, after a crash cut its recording short.
    let log = match std::fs::read(log_path) {
        Ok(raw) => String::from_utf8_lossy(&raw).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    for index in 0..record.transitions.len() {
        let seq = match record.transitions[index].event_id {
            Some(seq) => seq,
            None => {
                let seq = match emitted_seq(store, run_id, &record.transitions[index], index)? {
                    Some(seq) => seq,
                    None => emit_transition(store, run_id, &record.transitions[index], index)?,
                };
                record.transitions[index].event_id = Some(seq);
                store.write_run_json(run_id, transitions::TRANSITIONS_PATH, &record)?;
                seq
            }
        };
        let transition = &record.transitions[index];
        let key = format!("event_id={seq} transition={}", transition.label);
        if !log
            .lines()
            .any(|line| line == key || line.starts_with(&format!("{key} ")))
        {
            crate::command::workflow_decompose_log::append_nofollow_line(
                log_path,
                &transition.log_line(seq),
            )?;
        }
    }
    Ok(changed
        .then(|| record.transitions.last().cloned())
        .flatten())
}

/// The kind stays `BinaryRevisionDrift`, which every reader of the event log
/// (an older binary after a rollback too) parses; `detail.event` names the
/// transition.
fn emit_transition(
    store: &WorkflowStore,
    run_id: &str,
    transition: &RuntimeTransition,
    index: usize,
) -> Result<u64> {
    let seq = store.next_event_seq(run_id)?;
    archon_workflow::WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        archon_workflow::WorkflowEventKind::BinaryRevisionDrift,
        serde_json::json!({
            "event": transition.label,
            "transition_index": index,
            "old": transition.old,
            "new": transition.new,
            "persisted": transition.old.starting_binary_revision,
            "current": transition.new.starting_binary_revision,
        }),
    )?;
    std::fs::File::open(store.events_path(run_id))?.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(store.run_dir(run_id))?.sync_all()?;
    Ok(seq)
}

fn read_transitions(store: &WorkflowStore, run_id: &str) -> Result<Option<RuntimeTransitions>> {
    let path = store.run_dir(run_id).join(transitions::TRANSITIONS_PATH);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unmapped(transitions::TRANSITIONS_PATH, &error.to_string())),
    };
    let value: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| unmapped(transitions::TRANSITIONS_PATH, &e.to_string()))?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(u64::from(transitions::TRANSITIONS_SCHEMA_VERSION))
    {
        return Err(unmapped(
            &format!("{}.schema_version", transitions::TRANSITIONS_PATH),
            &format!(
                "found {}; this binary reads schema {}",
                value["schema_version"],
                transitions::TRANSITIONS_SCHEMA_VERSION
            ),
        ));
    }
    decode(value, transitions::TRANSITIONS_PATH).map(Some)
}

/// The seq of the event that already shows transition `index`. A line that
/// does not parse is skipped, as `next_event_seq` counts it: an interrupted
/// append of any event never blocks a resume.
fn emitted_seq(
    store: &WorkflowStore,
    run_id: &str,
    transition: &RuntimeTransition,
    index: usize,
) -> Result<Option<u64>> {
    let raw = match std::fs::read(store.events_path(run_id)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(raw
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .find(|event| {
            event["detail"]["event"] == transition.label.as_str()
                && event["detail"]["transition_index"] == serde_json::json!(index)
        })
        .and_then(|event| event["seq"].as_u64()))
}

/// Name the first unmappable field rather than silently substituting defaults.
pub(super) fn require_equal(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    field: &str,
) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    if let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) {
        for key in actual.keys().chain(expected.keys()) {
            if actual.contains_key(key) != expected.contains_key(key) {
                return Err(unmapped(
                    &format!("{field}.{key}"),
                    "missing or unexpected launch-bound field",
                ));
            }
            require_equal(
                actual.get(key).unwrap_or(&serde_json::Value::Null),
                expected.get(key).unwrap_or(&serde_json::Value::Null),
                &format!("{field}.{key}"),
            )?;
        }
    }
    Err(unmapped(
        field,
        "differs from the verified launch snapshot; restore the launch-bound input/configuration before resume",
    ))
}
