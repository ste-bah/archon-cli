//! Upgrade admission without mutating the immutable launch snapshot.
use super::*;
use crate::command::workflow_decompose_transitions::{
    self as transitions, RuntimeTransition, RuntimeTransitions,
};

/// A result-store file's bytes, read as the store reads them (Issue-292): a
/// FIFO or other refused entry fails the check, naming the file, instead of
/// blocking it.
fn read_named(path: &Path) -> Result<Vec<u8>> {
    archon_workflow::v2::store_file::read_store_file(path)
        .map_err(|error| anyhow!("{}: {error}", path.display()))
}

/// [`read_named`] for a call record: `None` for an entry the store's readers
/// refuse (a FIFO or other non-regular entry, a link out of its directory,
/// a link loop, an oversized file). The store itself moves such a slot into
/// `results/history/` when a new execution claims it, and its readers treat
/// it as a gap in that call's history, never as a record. So the check does
/// too: the refusal is reported (by `store_file`, naming the entry), it is
/// never counted, and it never blocks a resume.
fn read_record_or_gap(path: &Path) -> Result<Option<Vec<u8>>> {
    use archon_workflow::v2::store_file::{read_store_file, store_file_refusal};
    match read_store_file(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if store_file_refusal(&error).is_some() => Ok(None),
        Err(error) => Err(anyhow!("{}: {error}", path.display())),
    }
}

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
        let value = serde_json::from_slice(&read_named(&checkpoint)?)
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
        // A `.json` entry is a record candidate whatever its kind: a slot
        // the store moved aside unread can be a directory. A store
        // directory (`history/`, a call's archive) never has that name.
        let candidate = file
            .extension()
            .is_some_and(|extension| extension == "json");
        if !candidate && entry.file_type()?.is_dir() {
            if entry.file_name() != "quarantine" {
                validate_call_directory(&file, store, false)?;
            }
        } else if candidate {
            let Some(bytes) = read_record_or_gap(&file)? else {
                continue;
            };
            let field = file.display().to_string();
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| unmapped(&field, &e.to_string()))?;
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
/// identity and bundle are never replaced. A transition is recorded once every
/// launch-bound input has been verified; it is marked started only when an
/// executor takes the run ([`mark_started`]). Returns the transitions whose
/// event this call wrote, for the operator.
pub(super) fn record_upgrade(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    launch: &FixedRunIdentityV1,
    current: &FixedRunIdentityV1,
) -> Result<Vec<RuntimeTransition>> {
    let mut record = read_transitions(store, run_id)?.unwrap_or_default();
    let previous = record
        .transitions
        .last()
        .map_or_else(|| launch.clone(), |last| last.new.clone());
    if !transitions::same_runtime(&previous, current) {
        record
            .transitions
            .push(RuntimeTransition::new(previous, current.clone()));
        write_transitions(store, run_id, &record)?;
    }
    // The visible copies of every transition, each written once: normally
    // only the last one lacks any, after a crash cut its recording short.
    let log = match std::fs::read(log_path) {
        Ok(raw) => String::from_utf8_lossy(&raw).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    // A line torn by any writer must not swallow the next one.
    let mut torn_tail = !log.is_empty() && !log.ends_with('\n');
    let mut shown = Vec::new();
    for index in 0..record.transitions.len() {
        let seq = match record.transitions[index].event_id {
            Some(seq) => seq,
            None => {
                let transition = &record.transitions[index];
                let seq = match emitted_seq(store, run_id, transition, index)? {
                    Some(seq) => seq,
                    None => {
                        shown.push(transition.clone());
                        emit_transition(store, run_id, transition, index)?
                    }
                };
                record.transitions[index].event_id = Some(seq);
                write_transitions(store, run_id, &record)?;
                seq
            }
        };
        let transition = &record.transitions[index];
        let key = format!("event_id={seq} transition={}", transition.label);
        if !log
            .lines()
            .any(|line| line == key || line.starts_with(&format!("{key} ")))
        {
            let line = transition.log_line(seq);
            crate::command::workflow_decompose_log::append_nofollow_line(
                log_path,
                &if torn_tail { format!("\n{line}") } else { line },
            )?;
            torn_tail = false;
        }
    }
    Ok(shown)
}

/// Marks the last transition started: every resume check passed and this
/// process is the run's executor.
pub(super) fn mark_started(store: &WorkflowStore, run_id: &str) -> Result<()> {
    let Some(mut record) = read_transitions(store, run_id)? else {
        return Ok(());
    };
    match record.transitions.last_mut() {
        Some(last) if last.started_at.is_none() => {
            last.started_at = Some(chrono::Utc::now().to_rfc3339());
            write_transitions(store, run_id, &record)
        }
        _ => Ok(()),
    }
}

/// The record, replaced atomically and durably: the rename is synced into
/// its directory.
fn write_transitions(
    store: &WorkflowStore,
    run_id: &str,
    record: &RuntimeTransitions,
) -> Result<()> {
    store.write_run_json(run_id, transitions::TRANSITIONS_PATH, record)?;
    crate::command::workflow_task_set::sync_parent(
        &store.run_dir(run_id).join(transitions::TRANSITIONS_PATH),
    )
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
    crate::command::workflow_task_set::sync_file(&store.events_path(run_id))?;
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
                // The same transition, not another one at the same index
                // whose record was lost: the runtime it moved to matches.
                && serde_json::from_value::<FixedRunIdentityV1>(event["detail"]["new"].clone())
                    .is_ok_and(|new| transitions::same_runtime(&new, &transition.new))
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

#[cfg(all(test, unix))]
mod store_read_tests {
    use super::*;
    use std::path::PathBuf;

    fn mkfifo(path: &Path) {
        let raw = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0, "{path:?}");
    }

    /// A run's `v2/` with `results/history/<stem>/`, as the store archives
    /// a slot a new execution claims.
    fn store_dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("results");
        let history = results.join("history").join("call-x-0a1b");
        std::fs::create_dir_all(&history).unwrap();
        (dir, results, history)
    }

    /// The check over `results`, on its own thread: a check that blocks on
    /// a FIFO fails the test instead of hanging the suite.
    fn check(dir: &tempfile::TempDir, results: &Path) -> std::result::Result<(), String> {
        let store = archon_workflow::WorkflowV2ResultStore::new(dir.path());
        let (done, wait) = std::sync::mpsc::channel();
        let at = results.to_path_buf();
        std::thread::spawn(move || {
            let checked = validate_call_directory(&at, &store, true).map_err(|e| e.to_string());
            let _ = done.send(checked);
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("the check blocked on a FIFO")
    }

    /// Issue-292 L5: a FIFO the store moved aside into a call's history,
    /// or one in a slot, is a gap: the check returns at once and passes.
    #[test]
    fn a_fifo_in_the_result_store_is_a_gap_never_a_block() {
        let (dir, results, history) = store_dirs();
        mkfifo(&history.join("call-x-0a1b-20261007T230531Z-1.json"));
        mkfifo(&results.join("planted.json"));
        check(&dir, &results).expect("a refused entry blocked the resume");
    }

    /// Every other kind the store refuses is a gap too: a link out of its
    /// directory, a link loop, an oversized file, and a directory slot the
    /// store moved aside (whose contents are never records).
    #[test]
    fn every_refused_kind_in_history_is_a_gap_never_a_block() {
        let (dir, results, history) = store_dirs();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, b"{\"schema_version\":\"future\"}").unwrap();
        std::os::unix::fs::symlink(&outside, history.join("out.json")).unwrap();
        std::os::unix::fs::symlink("loop.json", history.join("loop.json")).unwrap();
        let big = std::fs::File::create(history.join("big.json")).unwrap();
        big.set_len(archon_workflow::v2::store_file::MAX_STORE_FILE_BYTES + 1)
            .unwrap();
        let moved = history.join("call-x-0a1b-20261007T230531Z-2.json");
        std::fs::create_dir(&moved).unwrap();
        std::fs::write(moved.join("junk.json"), b"not json").unwrap();
        check(&dir, &results).expect("a refused entry blocked the resume");
    }

    /// A gap is never counted, and never hides a record: refused entries in
    /// the legacy flat archive and in a slot pass, and a corrupt record
    /// beside them still fails, naming the record, not the gap.
    #[test]
    fn a_gap_in_the_flat_archive_passes_and_never_hides_a_corrupt_record() {
        let (dir, results, history) = store_dirs();
        let flat = results.join("superseded");
        std::fs::create_dir(&flat).unwrap();
        let fifo = flat.join("planted.json");
        mkfifo(&fifo);
        std::os::unix::fs::symlink("slot.json", results.join("slot.json")).unwrap();
        check(&dir, &results).expect("a refused entry blocked the resume");
        let corrupt = history.join("corrupt.json");
        std::fs::write(&corrupt, b"{\"schema_version\":\"future\"}").unwrap();
        let error = check(&dir, &results).expect_err("a corrupt record passed");
        assert!(error.contains(&corrupt.display().to_string()), "{error}");
        assert!(!error.contains(&fifo.display().to_string()), "{error}");
    }
}
