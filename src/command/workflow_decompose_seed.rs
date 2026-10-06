//! Phase seeds: where a resume after a runtime upgrade starts (Issue 360).
//!
//! Issue 358 reuses a recorded call only when its own input still matches.
//! That keeps almost nothing once the script or its prompts change: each
//! acceptance author prompt carries every other entry and earlier findings,
//! so one changed decision changes every later input. What is worth keeping
//! is the phase product, not the call sequence: the latest candidate, which
//! gates judged it and how, and the replies authored since.
//!
//! At each script, catalog or template transition the resume derives that
//! from the run's own records (`workflow_decompose_seed_derive`) and writes
//! it once to `decomposition/phase-seeds/transition-<index>.json`. Every
//! later resume on that runtime reads the same record back, so the seeded
//! attempts replay from their own records; a later transition derives a new
//! one. The old calls stay on disk as evidence and are no longer replayed:
//! the seeded calls continue every call and pause ordinal after them. The
//! seed reaches the script as `args.phaseSeed`; the launch-bound arguments
//! on disk never carry it.
//!
//! Visible as one event (kind `BinaryRevisionDrift`, `detail.event =
//! decomposition_phase_seeded`), one `.decompose.log` line and the status
//! lines below, each written once per seed.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore};
use serde::{Deserialize, Serialize};

use crate::command::workflow_decompose_transitions::{self as transitions, RuntimeTransitions};
pub(crate) use derive::SubjectSeed;

pub(crate) const SEEDS_DIR: &str = "decomposition/phase-seeds";
pub(crate) const SEED_SCHEMA_VERSION: u32 = 1;
pub(crate) const SEED_EVENT: &str = "decomposition_phase_seeded";

pub(crate) fn seed_unmapped(field: &str, reason: &str) -> anyhow::Error {
    crate::command::workflow_decompose::unmapped(field, reason)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhaseSeed {
    pub(crate) schema_version: u32,
    /// The runtime transition this seed starts.
    pub(crate) transition_index: usize,
    pub(crate) derived_at: String,
    /// The highest call ordinal each author subject's records hold.
    pub(crate) author_ordinals: BTreeMap<String, u64>,
    /// The highest pause ordinal each pause subject took.
    pub(crate) pause_ordinals: BTreeMap<String, u64>,
    pub(crate) subjects: BTreeMap<String, SubjectSeed>,
    /// The run's generation when the seed was derived: every pause taken
    /// before it covers history the seeded run no longer replays.
    pub(crate) pause_generation_floor: u64,
    /// The seq of the visible event, once written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) event_id: Option<u64>,
}

fn seed_path(index: usize) -> String {
    format!("{SEEDS_DIR}/transition-{index}.json")
}

/// The index of the last transition that changed the script, catalog or
/// template. A binary revision change alone keeps the script and its exact
/// replay, so it starts no seed and keeps the one before it.
fn seeded_transition(store: &WorkflowStore, run_id: &str) -> Result<Option<usize>> {
    let path = store.run_dir(run_id).join(transitions::TRANSITIONS_PATH);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(seed_unmapped(
                transitions::TRANSITIONS_PATH,
                &error.to_string(),
            ));
        }
    };
    let record: RuntimeTransitions = serde_json::from_slice(&raw)
        .map_err(|error| seed_unmapped(transitions::TRANSITIONS_PATH, &error.to_string()))?;
    Ok(record
        .transitions
        .iter()
        .rposition(|t| transitions::harness_changed(&t.old, &t.new)))
}

fn read_seed(store: &WorkflowStore, run_id: &str, index: usize) -> Result<Option<PhaseSeed>> {
    let relative = seed_path(index);
    let raw = match std::fs::read(store.run_dir(run_id).join(&relative)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(seed_unmapped(&relative, &error.to_string())),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| seed_unmapped(&relative, &e.to_string()))?;
    if value["schema_version"] != SEED_SCHEMA_VERSION {
        return Err(seed_unmapped(
            &format!("{relative}.schema_version"),
            &format!(
                "found {}; this binary reads schema {SEED_SCHEMA_VERSION}",
                value["schema_version"]
            ),
        ));
    }
    crate::command::workflow_decompose::decode(value, &relative).map(Some)
}

/// Replaced atomically and durably, as the transition record is.
fn write_seed(store: &WorkflowStore, run_id: &str, seed: &PhaseSeed) -> Result<()> {
    let relative = seed_path(seed.transition_index);
    store.write_run_json(run_id, &relative, seed)?;
    crate::command::workflow_task_set::sync_parent(&store.run_dir(run_id).join(relative))
}

/// The pause ids the run took, from their records.
fn taken_pauses(store: &WorkflowStore, run_id: &str) -> Result<Vec<String>> {
    let dir = store.run_dir(run_id).join("v2/script-pauses");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)
                .map_err(|e| seed_unmapped(&path.display().to_string(), &e.to_string()))?;
            let id = value["pause_id"]
                .as_str()
                .ok_or_else(|| seed_unmapped(&format!("{}.pause_id", path.display()), "missing"))?;
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn derive_seed(
    store: &WorkflowStore,
    run_id: &str,
    index: usize,
    criteria: &BTreeSet<String>,
) -> Result<PhaseSeed> {
    let records =
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")).load_call_records()?;
    let derived = derive::derive(&records, &taken_pauses(store, run_id)?, criteria)?;
    let generation = store.load_state(run_id)?.generation;
    Ok(PhaseSeed {
        pause_generation_floor: generation,
        schema_version: SEED_SCHEMA_VERSION,
        transition_index: index,
        derived_at: chrono::Utc::now().to_rfc3339(),
        author_ordinals: derived.author_ordinals,
        pause_ordinals: derived.pause_ordinals,
        subjects: derived.subjects,
        event_id: None,
    })
}

/// The run's seed for its current runtime, derived and recorded the first
/// time a resume reaches it; `None` before any upgrade.
pub(crate) fn current_seed(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    criteria: &BTreeSet<String>,
) -> Result<Option<PhaseSeed>> {
    let Some(index) = seeded_transition(store, run_id)? else {
        return Ok(None);
    };
    let mut seed = match read_seed(store, run_id, index)? {
        Some(seed) => seed,
        None => {
            let seed = derive_seed(store, run_id, index, criteria)?;
            write_seed(store, run_id, &seed)?;
            seed
        }
    };
    if seed.event_id.is_none() {
        seed.event_id = Some(emit_event(store, run_id, &seed)?);
        write_seed(store, run_id, &seed)?;
    }
    record_log_line(log_path, &seed)?;
    Ok(Some(seed))
}

/// `arguments` with the seed the script starts from.
pub(crate) fn seeded_arguments(
    arguments: &serde_json::Value,
    seed: Option<&PhaseSeed>,
) -> serde_json::Value {
    let mut arguments = arguments.clone();
    if let (Some(seed), Some(map)) = (seed, arguments.as_object_mut()) {
        map.insert(
            "phaseSeed".into(),
            serde_json::json!({
                "transition_index": seed.transition_index,
                "author_ordinals": seed.author_ordinals,
                "pause_ordinals": seed.pause_ordinals,
                "pause_generation_floor": seed.pause_generation_floor,
                "subjects": seed.subjects,
            }),
        );
    }
    arguments
}

/// One line per subject: what the seed carries and what judged it last.
fn subject_lines(seed: &PhaseSeed) -> Vec<String> {
    seed.subjects
        .iter()
        .map(|(subject, carried)| match carried {
            SubjectSeed::Entries { gates, replies, invalid, carried, .. } => format!(
                "{subject}: carried_entries={carried} replies_since_gate={} invalid_under_this_build={} last_gate={} last_gate_findings={}",
                replies.len(),
                invalid.len(),
                gates.last().map_or("none", |g| g.call_id.as_str()),
                gates.last().map_or(0, |g| g.findings.len()),
            ),
            SubjectSeed::Artifact { candidate_call, gate, .. } => format!(
                "{subject}: candidate={candidate_call} last_gate={} last_gate_findings={}",
                gate.as_ref().map_or("none", |g| g.call_id.as_str()),
                gate.as_ref().map_or(0, |g| g.findings.len()),
            ),
        })
        .collect()
}

fn emit_event(store: &WorkflowStore, run_id: &str, seed: &PhaseSeed) -> Result<u64> {
    // Written before a crash cut the record's event id short: never twice.
    if let Some(seq) = existing_event(store, run_id, seed.transition_index)? {
        return Ok(seq);
    }
    let seq = store.next_event_seq(run_id)?;
    archon_workflow::WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        archon_workflow::WorkflowEventKind::BinaryRevisionDrift,
        serde_json::json!({
            "event": SEED_EVENT,
            "transition_index": seed.transition_index,
            "record": seed_path(seed.transition_index),
            "subjects": subject_lines(seed),
        }),
    )?;
    std::fs::File::open(store.events_path(run_id))?.sync_all()?;
    Ok(seq)
}

fn existing_event(store: &WorkflowStore, run_id: &str, index: usize) -> Result<Option<u64>> {
    let raw = match std::fs::read(store.events_path(run_id)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(raw
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .find(|event| {
            event["detail"]["event"] == SEED_EVENT
                && event["detail"]["transition_index"] == serde_json::json!(index)
        })
        .and_then(|event| event["seq"].as_u64()))
}

fn record_log_line(log_path: &Path, seed: &PhaseSeed) -> Result<()> {
    let Some(seq) = seed.event_id else {
        return Ok(());
    };
    let key = format!("event_id={seq} transition={SEED_EVENT}");
    let log = match std::fs::read(log_path) {
        Ok(raw) => String::from_utf8_lossy(&raw).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if log
        .lines()
        .any(|line| line == key || line.starts_with(&format!("{key} ")))
    {
        return Ok(());
    }
    let field = crate::command::workflow_decompose_events::log_field;
    let line = format!(
        "{key} index={} subjects={}",
        seed.transition_index,
        field(&subject_lines(seed).join("; "))
    );
    let torn = !log.is_empty() && !log.ends_with('\n');
    crate::command::workflow_decompose_log::append_nofollow_line(
        log_path,
        &if torn { format!("\n{line}") } else { line },
    )?;
    Ok(())
}

/// The status lines for the run's current seed: none before any upgrade.
pub(crate) fn status_lines(store: &WorkflowStore, run_id: &str) -> String {
    let index = match seeded_transition(store, run_id) {
        Ok(Some(index)) => index,
        Ok(None) => return String::new(),
        Err(error) => return format!("phase_seed: unreadable ({error:#})\n"),
    };
    match read_seed(store, run_id, index) {
        Ok(Some(seed)) => {
            let mut out = format!(
                "phase_seed: transition={} record={} subjects={}\n",
                seed.transition_index,
                seed_path(seed.transition_index),
                seed.subjects.len()
            );
            for line in subject_lines(&seed) {
                out.push_str(&format!("- seed {line}\n"));
            }
            out
        }
        Ok(None) => {
            format!("phase_seed: transition={index} not yet derived (the next resume derives it)\n")
        }
        Err(error) => format!("phase_seed: unreadable ({error:#})\n"),
    }
}

#[path = "workflow_decompose_seed_derive.rs"]
mod derive;

#[cfg(test)]
#[path = "workflow_decompose_seed_tests.rs"]
pub(crate) mod tests;
