//! The write half of the SONA loop on `[workflow.generated]`.
//!
//! Turns one finished run into at most four observations — the budget pressure
//! each tunable parameter was under — and offers them to the tuner, which is
//! what decides whether they may be persisted.
//!
//! # Why the two families of parameter are derived differently
//!
//! The iteration budgets and the timeout budgets fail in opposite ways.
//!
//! An unused iteration budget is pure cost: the loop never entered, so a
//! smaller cap would have produced the identical run. That is real evidence for
//! shrinking, and it is recorded as such.
//!
//! An unused timeout budget is *not* evidence for shrinking. "No branch timed
//! out" says the budget was not too small; it says nothing about how close any
//! branch came. The observed incident is exactly this mistake made by hand — a
//! 1200s verification timeout looked generous until a verifier needed longer,
//! timed out, and VOIDED an already-accepted remediation. So the timeout
//! parameters are recorded as a **ratchet**: a clean run records the neutral
//! pressure 0.5, which counts as evidence without moving the weight, and only
//! an observed timeout records 1.0. No sequence of successful runs can shorten
//! a timeout, which is the property that makes that incident unreachable
//! through learning rather than merely unlikely.

use std::path::Path;

use archon_core::config::{LearningConfig, TunableGeneratedParameter};
use archon_pipeline::learning::sona::{AdmissionOutcome, SonaParameterTuner, TuningObservation};
use archon_pipeline::learning::trajectory_store;
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore};

use super::{learning_store_path, load_observations, sona_tuning_enabled};

/// Neutral pressure: evidence that a budget was observed, with no opinion on
/// its size. `calculate_gradient` subtracts 0.5, so this moves no weight.
const NEUTRAL_PRESSURE: f64 = 0.5;

/// Record what this run's generated limits were actually under.
///
/// Best-effort in the same sense as the rest of the fold: an unreadable run, an
/// unopenable store, or a rejected batch all return without changing what the
/// user's run reported. Recording an observation must never alter a result.
pub(crate) fn record_generated_tuning_outcome(
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
        |error| tracing::debug!(%error, "learning store unavailable; tuning outcome not recorded"),
    ) else {
        return;
    };
    if let Err(error) = archon_pipeline::learning::schema::initialize_learning_schemas(&db) {
        tracing::debug!(%error, "learning schema init failed; tuning outcome not recorded");
        return;
    }

    let mut tuner = SonaParameterTuner::from_history(class, &load_observations(&db, class));
    if let AdmissionOutcome::DriftRejected(report) = tuner.admit(&observations) {
        // Constraint: a tuner that can drift without rolling back is worse than
        // a static default. The rollback already happened inside `admit`; the
        // consequence here is that the batch is never written, so the next run
        // replays a history this outlier is not part of.
        tracing::warn!(
            class,
            run_id,
            divergence = report.divergence,
            threshold = report.threshold_used,
            "SONA tuning batch diverged past the reject threshold; rolled back and discarded"
        );
        return;
    }

    let recorded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    for observation in &observations {
        let route = SonaParameterTuner::route(class, &observation.parameter_key);
        let trajectory = tuning_trajectory(&route, run_id, observation, recorded_at);
        if let Err(error) = trajectory_store::store_trajectory(&db, &trajectory) {
            tracing::debug!(%error, %route, "tuning observation not persisted");
        }
    }
}

/// One persisted observation row.
///
/// `quality` carries the pressure and `reward` the value SONA's own
/// `calculate_reward` would produce for it, so a row read back and replayed
/// reproduces the weight exactly. The embedding is left empty on purpose: these
/// rows describe a config value, not a piece of work, and feeding them to the
/// GNN trainer would train it on rows that have no work in them.
fn tuning_trajectory(
    route: &str,
    run_id: &str,
    observation: &TuningObservation,
    recorded_at: u64,
) -> archon_pipeline::learning::sona::Trajectory {
    archon_pipeline::learning::sona::Trajectory {
        trajectory_id: format!("tuning-{run_id}-{}", observation.parameter_key),
        route: route.to_string(),
        agent_key: "generated-workflow-tuner".to_string(),
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

/// Derive this run's budget pressures from what it persisted.
///
/// Returns an empty vector when the run left nothing to read. A parameter with
/// no attributable evidence is simply absent — inventing a neutral row for it
/// would inflate the observation count that gates the whole loop.
fn observe_run(store: &WorkflowStore, run_id: &str) -> Vec<TuningObservation> {
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let Ok(records) = v2_store.load_call_records() else {
        return Vec::new();
    };
    if records.is_empty() {
        return Vec::new();
    }

    let resolved = store
        .load_state(run_id)
        .is_ok_and(|run| matches!(run.status, archon_workflow::RunStatus::Completed));
    let mut observations = Vec::new();
    let mut saw_repair = false;
    let mut saw_investigation = false;
    let mut verification_timeout = false;
    let mut host_timeout = false;

    for record in &records {
        saw_repair |= is_repair_call(&record.call.id);
        saw_investigation |= is_investigation_call(&record.call.id);
        for outcome in record
            .result
            .data
            .get("outcomes")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if !is_timeout_error(outcome.get("error").and_then(serde_json::Value::as_str)) {
                continue;
            }
            if is_verification_call(&record.call.id) {
                verification_timeout = true;
            } else {
                host_timeout = true;
            }
        }
    }

    if let Some(pressure) = iteration_pressure(resolved, saw_repair) {
        observations.push(observation(
            TunableGeneratedParameter::MaxRepairIterations,
            pressure,
        ));
    }
    if let Some(pressure) = iteration_pressure(resolved, saw_investigation) {
        observations.push(observation(
            TunableGeneratedParameter::MaxInvestigationIterations,
            pressure,
        ));
    }
    observations.push(observation(
        TunableGeneratedParameter::VerificationBranchTimeoutSecs,
        ratchet_pressure(verification_timeout),
    ));
    observations.push(observation(
        TunableGeneratedParameter::HostCallTimeoutSecs,
        ratchet_pressure(host_timeout),
    ));
    observations
}

fn observation(parameter: TunableGeneratedParameter, pressure: f64) -> TuningObservation {
    TuningObservation {
        parameter_key: parameter.key().to_string(),
        pressure,
        recorded_at: 0,
    }
}

/// How hard this run leaned on an iteration budget.
///
/// - Resolved without entering the loop: the cap was never reached and a
///   smaller one would have produced the same run. Pressure 0.
/// - Resolved after entering the loop: the cap was used and sufficed. Neutral.
/// - Unresolved after entering the loop: the loop ran and still did not
///   converge, and the cap is the suspect. Pressure 1. This over-attributes —
///   some of those runs were unresolvable at any cap — which is why the ceiling
///   in `archon_core::config::generated_tuning` exists and why it takes double
///   figures of consistent runs to move one step.
/// - Unresolved without entering the loop: the run failed for a reason this
///   budget cannot explain. `None` — no observation, no evidence count.
fn iteration_pressure(resolved: bool, entered_loop: bool) -> Option<f64> {
    match (resolved, entered_loop) {
        (true, false) => Some(0.0),
        (true, true) => Some(NEUTRAL_PRESSURE),
        (false, true) => Some(1.0),
        (false, false) => None,
    }
}

/// A timeout budget only ever ratchets up. See the module docs.
fn ratchet_pressure(timed_out: bool) -> f64 {
    if timed_out { 1.0 } else { NEUTRAL_PRESSURE }
}

/// Branch timeouts surface as an execution error string, not a typed failure
/// kind — `BranchFailureKind` has no timeout variant — so the string is the
/// only signal available. Matched on both spellings the runtime emits.
fn is_timeout_error(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        let error = error.to_ascii_lowercase();
        error.contains("timed out") || error.contains("timeout")
    })
}

/// Calls whose branches run under `verification_branch_timeout_secs`.
///
/// Mirrors the read-only branch timeout split in
/// `workflow_live_v2_read_only_b`: a verification or review branch gets the
/// verification budget, everything else gets the host-call budget. Attributing
/// a timeout to the wrong budget would lengthen a limit that was never the
/// constraint while leaving the real one untouched.
fn is_verification_call(call_id: &str) -> bool {
    call_id.contains("verification") || call_id.contains("verify") || call_id.contains("review")
}

fn is_repair_call(call_id: &str) -> bool {
    call_id.starts_with("remediation-inventory-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-inventory-")
        || call_id.starts_with("review-remediation-wave-")
        || call_id.starts_with("verification-remediation-inventory-")
}

fn is_investigation_call(call_id: &str) -> bool {
    call_id.contains("investigation")
}

#[cfg(test)]
#[path = "workflow_live_sona_tuning_outcome_tests.rs"]
mod tests;
