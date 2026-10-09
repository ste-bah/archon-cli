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
//! it once to `decomposition/phase-seeds/transition-<index>-<runtime>.json`,
//! named by the transition and the runtime it was derived for. Every later
//! resume on that runtime reads the same record back, so the seeded attempts
//! replay from their own records; a later transition derives a new one. A
//! seed is never used for another runtime: a transition record rebuilt with
//! another runtime at the same index names another file, derived afresh from
//! every record (the earlier seeded rounds included). The old calls stay on
//! disk as evidence and are no longer replayed: the seeded calls continue
//! every call and pause ordinal after them, and no call answers from an
//! attempt recorded before the seed unless the executor finds that landing
//! still live. The seed reaches the script as `args.phaseSeed`; the
//! launch-bound arguments on disk never carry it.
//!
//! Visible as one event (kind `BinaryRevisionDrift`, `detail.event =
//! decomposition_phase_seeded`), one `.decompose.log` line and the status
//! lines below, each written once per seed.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use archon_workflow::{FixedRunIdentityV1, WorkflowStore, WorkflowV2ResultStore};
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
    /// What that transition runs (its `new` identity): the seed is read back
    /// only for this script, catalog and template.
    pub(crate) runtime: FixedRunIdentityV1,
    pub(crate) derived_at: String,
    /// The highest call ordinal each author subject's records hold.
    pub(crate) author_ordinals: BTreeMap<String, u64>,
    /// The highest pause ordinal each pause subject took.
    pub(crate) pause_ordinals: BTreeMap<String, u64>,
    pub(crate) subjects: BTreeMap<String, SubjectSeed>,
    /// The run's generation when the seed was derived: every pause taken
    /// before it covers history the seeded run no longer replays.
    pub(crate) pause_generation_floor: u64,
    /// The attempt each call's record held when the seed was derived: no
    /// attempt at or below it answers the seeded run as history.
    pub(crate) history_attempts: BTreeMap<String, u32>,
    /// The seq of the visible event, once written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) event_id: Option<u64>,
}

/// The script, catalog and template a seed is for: what a binary revision
/// drift alone leaves unchanged.
fn runtime_key(runtime: &FixedRunIdentityV1) -> String {
    let harness = format!(
        "{}\n{}\n{}",
        runtime.template_version, runtime.script_digest, runtime.catalog_digest
    );
    archon_workflow::task_set_contract::content_digest(harness.as_bytes())[..16].to_string()
}

fn seed_path(index: usize, runtime: &FixedRunIdentityV1) -> String {
    format!(
        "{SEEDS_DIR}/transition-{index}-{}.json",
        runtime_key(runtime)
    )
}

/// The last transition that changed the script, catalog or template, with
/// what it runs. A binary revision change alone keeps the script and its
/// exact replay, so it starts no seed and keeps the one before it.
fn seeded_transition(
    store: &WorkflowStore,
    run_id: &str,
) -> Result<Option<(usize, FixedRunIdentityV1)>> {
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
        .rposition(|t| transitions::harness_changed(&t.old, &t.new))
        .map(|index| (index, record.transitions[index].new.clone())))
}

fn read_seed(
    store: &WorkflowStore,
    run_id: &str,
    index: usize,
    runtime: &FixedRunIdentityV1,
) -> Result<Option<PhaseSeed>> {
    let relative = seed_path(index, runtime);
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
    let seed: PhaseSeed = crate::command::workflow_decompose::decode(value, &relative)?;
    if seed.transition_index != index || transitions::harness_changed(&seed.runtime, runtime) {
        return Err(seed_unmapped(
            &format!("{relative}.runtime"),
            &format!(
                "the record was derived for transition {} on script {}, catalog {}, template {}; transition {index} runs script {}, catalog {}, template {}. It is never used for another runtime: move it aside and the next resume derives this runtime's seed",
                seed.transition_index,
                seed.runtime.script_digest,
                seed.runtime.catalog_digest,
                seed.runtime.template_version,
                runtime.script_digest,
                runtime.catalog_digest,
                runtime.template_version
            ),
        ));
    }
    Ok(Some(seed))
}

/// Replaced atomically and durably, as the transition record is. The seeds
/// directory's own entry is flushed into its parent on every write, not only
/// when this write creates it: a crash may have left it created but unflushed.
fn write_seed(store: &WorkflowStore, run_id: &str, seed: &PhaseSeed) -> Result<()> {
    use crate::command::workflow_task_set::{create_dir_all_durably, sync_parent};
    let relative = seed_path(seed.transition_index, &seed.runtime);
    let (run_dir, seeds) = (store.run_dir(run_id), store.run_dir(run_id).join(SEEDS_DIR));
    create_dir_all_durably(&seeds)?;
    sync_parent(&seeds)?;
    store.write_run_json(run_id, &relative, seed)?;
    sync_parent(&run_dir.join(relative))
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
    (index, runtime): (usize, FixedRunIdentityV1),
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
) -> Result<PhaseSeed> {
    let results = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let records = results.load_call_records()?;
    let derived = derive::derive_with_requirement_texts(
        &records,
        &taken_pauses(store, run_id)?,
        criteria,
        requirement_texts,
    )?;
    let generation = store.load_state(run_id)?.generation;
    // Archived attempts included: a slot may hold a restored older attempt.
    let mut history_attempts = BTreeMap::new();
    for record in &records {
        let next = results.next_attempt(&record.call.id)?;
        history_attempts.insert(record.call.id.clone(), next.saturating_sub(1));
    }
    Ok(PhaseSeed {
        pause_generation_floor: generation,
        history_attempts,
        schema_version: SEED_SCHEMA_VERSION,
        transition_index: index,
        runtime,
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
    criteria: &BTreeMap<String, String>,
) -> Result<Option<PhaseSeed>> {
    current_seed_with_requirement_texts(store, run_id, log_path, criteria, &BTreeMap::new())
}

pub(crate) fn current_seed_with_requirement_texts(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
) -> Result<Option<PhaseSeed>> {
    let Some((index, runtime)) = seeded_transition(store, run_id)? else {
        return Ok(None);
    };
    let mut seed = match read_seed(store, run_id, index, &runtime)? {
        Some(seed) => seed,
        None => {
            let seed = derive_seed(store, run_id, (index, runtime), criteria, requirement_texts)?;
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
                "history_attempts": seed.history_attempts,
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
    let record = seed_path(seed.transition_index, &seed.runtime);
    if let Some(seq) = existing_event(store, run_id, seed.transition_index, &record)? {
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
            "record": record,
            "runtime": seed.runtime,
            "subjects": subject_lines(seed),
        }),
    )?;
    crate::command::workflow_task_set::sync_file(&store.events_path(run_id))?;
    Ok(seq)
}

/// The seed event of transition `index` for the seed `record`, which names
/// the runtime: another runtime's seed at the same index is another event.
fn existing_event(
    store: &WorkflowStore,
    run_id: &str,
    index: usize,
    record: &str,
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
            event["detail"]["event"] == SEED_EVENT
                && event["detail"]["transition_index"] == serde_json::json!(index)
                && event["detail"]["record"] == record
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
    let (index, runtime) = match seeded_transition(store, run_id) {
        Ok(Some(found)) => found,
        Ok(None) => return String::new(),
        Err(error) => return format!("phase_seed: unreadable ({error:#})\n"),
    };
    match read_seed(store, run_id, index, &runtime) {
        Ok(Some(seed)) => {
            let mut out = format!(
                "phase_seed: transition={} record={} subjects={}\n",
                seed.transition_index,
                seed_path(seed.transition_index, &seed.runtime),
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
