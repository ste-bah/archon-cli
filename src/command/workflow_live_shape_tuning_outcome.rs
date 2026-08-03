//! The write half of the SONA loop on the generated plan's shape.
//!
//! Turns one finished run into at most one observation — how contended the
//! implementation fan-out was — and offers it to the tuner, which is what
//! decides whether it may be persisted.
//!
//! # Why this is a ratchet, in the opposite direction to Phase 7's timeouts
//!
//! Phase 7 ratchets its timeout budgets *upward*: a clean run records the
//! neutral pressure, and only an observed timeout records 1.0, so no sequence
//! of successful runs can shorten a timeout. The same argument applies here
//! mirrored, because the failure modes are mirrored.
//!
//! A run that finished with no contention is **not** evidence that a wider
//! fan-out would have been fine. The width was already at the operator's
//! configured cap; no branch ever tried to exceed it, so nothing in the run
//! tested whether more would have worked. Reading "no contention" as "widen"
//! would be concluding something no run ever measured — and the direction it
//! would conclude it in is the direction that corrupts worktrees rather than
//! the one that merely runs slowly.
//!
//! So: a clean run records the neutral pressure, which counts as evidence
//! without moving the weight, and only observed contention records 1.0. The
//! width can therefore only ever fall below the configured cap, never rise
//! above it — which is the property that makes this knob safe to learn at all.

use std::path::Path;

use archon_core::config::{LearningConfig, TunableShapeKnob};
use archon_pipeline::learning::sona::{AdmissionOutcome, SonaParameterTuner, TuningObservation};
use archon_pipeline::learning::trajectory_store;
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore};

use super::{load_shape_observations, sona_tuning_enabled};
use crate::command::workflow_live_sona_tuning::learning_store_path;

/// Neutral pressure: evidence that the fan-out was observed, with no opinion on
/// its width. `calculate_gradient` subtracts 0.5, so this moves no weight.
const NEUTRAL_PRESSURE: f64 = 0.5;

/// Record how contended this run's implementation fan-out was.
///
/// Best-effort in the same sense as the rest of the fold: an unreadable run, an
/// unopenable store, or a rejected batch all return without changing what the
/// user's run reported. Recording an observation must never alter a result.
pub(crate) fn record_generated_shape_outcome(
    project_root: &Path,
    store: &WorkflowStore,
    run_id: &str,
    class: &str,
    learning: &LearningConfig,
) {
    if !sona_tuning_enabled(learning) {
        return;
    }
    let observations = observe_run(store, run_id);
    if observations.is_empty() {
        return;
    }
    let path = learning_store_path(project_root);
    let Ok(db) = crate::command::topology_fold::open_store(&path, "learning").map_err(
        |error| tracing::debug!(%error, "learning store unavailable; shape outcome not recorded"),
    ) else {
        return;
    };
    if let Err(error) = archon_pipeline::learning::schema::initialize_learning_schemas(&db) {
        tracing::debug!(%error, "learning schema init failed; shape outcome not recorded");
        return;
    }

    let mut tuner = SonaParameterTuner::from_history(class, &load_shape_observations(&db, class));
    if let AdmissionOutcome::DriftRejected(report) = tuner.admit(&observations) {
        // A tuner that can drift without rolling back is worse than a static
        // default. The rollback already happened inside `admit`; the
        // consequence here is that the batch is never written, so the next run
        // replays a history this outlier is not part of.
        tracing::warn!(
            class,
            run_id,
            divergence = report.divergence,
            threshold = report.threshold_used,
            "SONA shape batch diverged past the reject threshold; rolled back and discarded"
        );
        return;
    }

    let recorded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    for observation in &observations {
        let route = SonaParameterTuner::route(class, &observation.parameter_key);
        let trajectory = shape_trajectory(&route, run_id, observation, recorded_at);
        if let Err(error) = trajectory_store::store_trajectory(&db, &trajectory) {
            tracing::debug!(%error, %route, "shape observation not persisted");
        }
    }
}

/// One persisted observation row.
///
/// `quality` carries the pressure and `reward` the value SONA's own
/// `calculate_reward` would produce for it, so a row read back and replayed
/// reproduces the weight exactly. The embedding is left empty on purpose: these
/// rows describe a topology decision, not a piece of work, and feeding them to
/// the GNN trainer would train it on rows that have no work in them.
fn shape_trajectory(
    route: &str,
    run_id: &str,
    observation: &TuningObservation,
    recorded_at: u64,
) -> archon_pipeline::learning::sona::Trajectory {
    archon_pipeline::learning::sona::Trajectory {
        trajectory_id: format!("shape-{run_id}-{}", observation.parameter_key),
        route: route.to_string(),
        agent_key: "generated-workflow-shape-tuner".to_string(),
        session_id: run_id.to_string(),
        patterns: Vec::new(),
        context: Vec::new(),
        embedding: Vec::new(),
        quality: observation.pressure,
        reward: observation.pressure,
        feedback_score: 1.0,
        weights_path: String::new(),
        created_at: recorded_at,
        updated_at: recorded_at,
    }
}

/// Derive this run's fan-out contention from what it persisted.
///
/// Returns an empty vector when the run left no write wave to read. A run that
/// never dispatched a write fan-out says nothing about how wide one should be,
/// and inventing a neutral row for it would inflate the observation count that
/// gates the whole loop.
fn observe_run(store: &WorkflowStore, run_id: &str) -> Vec<TuningObservation> {
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let Ok(records) = v2_store.load_call_records() else {
        return Vec::new();
    };

    let mut saw_write_wave = false;
    let mut contended = false;
    for record in &records {
        if !is_write_wave_call(&record.call.id) {
            continue;
        }
        saw_write_wave = true;
        for outcome in record
            .result
            .data
            .get("outcomes")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if is_contention_error(outcome.get("error").and_then(serde_json::Value::as_str)) {
                contended = true;
            }
        }
    }
    if !saw_write_wave {
        return Vec::new();
    }

    vec![TuningObservation {
        parameter_key: TunableShapeKnob::ImplementationWaveFanoutWidth
            .key()
            .to_string(),
        pressure: if contended { 1.0 } else { NEUTRAL_PRESSURE },
        recorded_at: 0,
    }]
}

/// The three write-capable fan-out families. Mirrors the `WORKTREE` write mode
/// in `workflow_live_generated_scaffold`: these are the only calls whose
/// branches hold a worktree, and therefore the only ones whose concurrency the
/// width knob governs. Read-only parallel stages share no writable state and
/// their contention would be attributed to the wrong knob.
fn is_write_wave_call(call_id: &str) -> bool {
    call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
}

/// Concurrency contention surfaces as an execution error string, not a typed
/// failure kind — `BranchFailureKind` has no contention variant — so the string
/// is the only signal available, exactly as it is for Phase 7's timeouts.
///
/// Matched narrowly on the wordings the worktree and write-coordination paths
/// emit. A false negative costs a missed observation and the knob stays at the
/// operator's cap; a false positive would narrow a fan-out that was fine, which
/// costs wall-clock and nothing else. The asymmetry is deliberate and is why
/// the list is not widened to catch every error mentioning "failed".
fn is_contention_error(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        let error = error.to_ascii_lowercase();
        error.contains("worktree lock")
            || error.contains("worktree is locked")
            || error.contains("already locked")
            || error.contains("lock contention")
            || error.contains("write conflict")
            || error.contains("conflicting write")
            || error.contains("index.lock")
    })
}

#[cfg(test)]
#[path = "workflow_live_shape_tuning_outcome_tests.rs"]
mod tests;
