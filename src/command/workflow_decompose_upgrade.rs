//! Upgrade admission without mutating the immutable launch snapshot.
use super::*;
use std::io::{BufRead, BufReader};

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

/// One synced event per runtime transition, even if preparation is retried.
/// Reading the last transition also records a rollback as an upgrade event.
/// Launch identity and bundle are never replaced.
pub(super) fn record_upgrade(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    launch: &FixedRunIdentityV1,
    current: &FixedRunIdentityV1,
) -> Result<bool> {
    let events_path = store.events_path(run_id);
    let mut previous = launch.clone();
    if events_path.exists() {
        for (index, line) in BufReader::new(std::fs::File::open(&events_path)?)
            .lines()
            .enumerate()
        {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
                unmapped(&format!("events.jsonl.line[{}]", index + 1), &e.to_string())
            })?;
            if matches!(
                event["detail"]["event"].as_str(),
                Some("decomposition_runtime_upgrade" | "binary_revision_drift")
            ) {
                if let Some(new) = event["detail"].get("new") {
                    previous = decode(new.clone(), "events.jsonl.detail.new")?;
                } else if let Some(revision) = event["detail"]["current"].as_str() {
                    // Explicit mapping of Issue 59's binary-only event.
                    previous.starting_binary_revision = revision.to_string();
                } else {
                    return Err(unmapped(
                        "events.jsonl.detail.current",
                        "binary drift event has no revision",
                    ));
                }
            }
        }
    }
    // Root identity was verified separately. Only runtime components define
    // an upgrade, including when public event sanitization redacts a path.
    if previous.template_version == current.template_version
        && previous.script_digest == current.script_digest
        && previous.catalog_digest == current.catalog_digest
        && previous.starting_binary_revision == current.starting_binary_revision
    {
        return Ok(false);
    }
    let harness_changed = previous.template_version != current.template_version
        || previous.script_digest != current.script_digest
        || previous.catalog_digest != current.catalog_digest;
    let label = if harness_changed {
        "decomposition_runtime_upgrade"
    } else {
        "binary_revision_drift"
    };
    let seq = store.next_event_seq(run_id)?;
    archon_workflow::WorkflowEventLog::new(store.clone()).emit(
        run_id, seq, archon_workflow::WorkflowEventKind::BinaryRevisionDrift,
        serde_json::json!({"event": label, "old": previous, "new": current,
            "persisted": previous.starting_binary_revision, "current": current.starting_binary_revision}),
    )?;
    std::fs::File::open(&events_path)?.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(store.run_dir(run_id))?.sync_all()?;
    crate::command::workflow_decompose_log::append_nofollow_line(
        log_path,
        &format!(
            "event_id={seq} transition={label} persisted={} current={}",
            crate::command::workflow_decompose_events::log_field(
                &previous.starting_binary_revision
            ),
            crate::command::workflow_decompose_events::log_field(&current.starting_binary_revision),
        ),
    )?;
    Ok(true)
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
