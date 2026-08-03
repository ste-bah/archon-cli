//! The read half of the SONA loop on `[workflow.generated]`.
//!
//! # What was missing
//!
//! SONA recorded and nothing read. `derive_learning_hooks` emitted the name
//! `"sona"` for a run, the fold wrote trajectories, and `get_weight` had zero
//! callers outside its own crate. A learner whose output nothing consumes is a
//! log with extra maths. This module is the first consumer: it asks for a
//! weight per tunable parameter, hands it to
//! [`archon_core::config::apply_generated_tuning`] — which owns the bounds —
//! and reports what it did.
//!
//! # Keying
//!
//! One weight per `(task class, parameter)`. The class comes from
//! [`crate::command::workflow_live_learning_hooks::classify_generated_run`],
//! the same classification that already decides the run's learning hooks, so a
//! run is classified once and everything keyed on that classification agrees.
//! Project identity is *not* part of the key: the store is already per-project
//! (`.archon/learning-state.db`), so adding it would only make the key longer.
//!
//! # What this module must never touch
//!
//! Phase 6. Requirement satisfaction is binary and evidence-anchored; a weight
//! is a continuous number derived from how previous runs went. Letting one
//! reach the other reproduces finding F1 — a padded gap report claiming
//! requirements were mapped — with better arithmetic behind it and therefore
//! more credibility. Nothing in `archon-knowledge` or `requirement_trace*` may
//! read a SONA weight, and
//! `workflow_live_sona_tuning_isolation_tests.rs` fails the build if one
//! starts to. The only thing this module is allowed to change is how long a
//! branch may run and how many times a loop may retry.

use std::path::Path;

use archon_core::config::{
    GeneratedTuningDecision, GeneratedTuningInput, GeneratedWorkflowConfig, LearningConfig,
    TunableGeneratedParameter, TuningSource, apply_generated_tuning,
};
use archon_pipeline::learning::sona::{SonaParameterTuner, TuningObservation};
use archon_pipeline::learning::trajectory_store;

#[path = "workflow_live_sona_tuning_outcome.rs"]
mod workflow_live_sona_tuning_outcome;
pub(crate) use workflow_live_sona_tuning_outcome::record_generated_tuning_outcome;

#[cfg(test)]
#[path = "workflow_live_sona_tuning_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_live_sona_tuning_isolation_tests.rs"]
mod isolation_tests;

/// Path of the per-project learning store, matching the one the learning fold
/// opens. Kept as one function so the tuner cannot drift onto a second store
/// and quietly learn from an empty one.
pub(crate) fn learning_store_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".archon").join("learning-state.db")
}

/// Whether the operator has consented to SONA learning from batch runs.
///
/// Identical to the gate `derive_learning_hooks` applies before emitting the
/// `"sona"` hook: `enabled` alone covers interactive sessions, and a workflow
/// run is a batch run. Reading a weight is gated on the same consent as writing
/// one, because a project that never consented to recording has no evidence and
/// would get baselines anyway — failing the gate here just makes that explicit
/// instead of opening a store to discover it.
pub(crate) fn sona_tuning_enabled(learning: &LearningConfig) -> bool {
    learning.sona.enabled && learning.sona.pipeline_recording
}

/// Tuned config plus the decisions that produced it.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedTuning {
    pub(crate) config: GeneratedWorkflowConfig,
    pub(crate) decisions: Vec<GeneratedTuningDecision>,
}

impl GeneratedTuning {
    /// The untouched baseline, with no decisions to report.
    ///
    /// Used whenever SONA is off or the store will not open. Reporting no
    /// decisions rather than four `Baseline` ones keeps the run output silent
    /// in the overwhelmingly common case where nothing was learned.
    pub(crate) fn untuned(config: GeneratedWorkflowConfig) -> Self {
        Self {
            config,
            decisions: Vec::new(),
        }
    }

    /// True when at least one parameter differs from the operator's config.
    pub(crate) fn moved(&self) -> bool {
        self.decisions
            .iter()
            .any(|decision| decision.source.moved() && decision.applied != decision.baseline)
            || self
                .decisions
                .iter()
                .any(|decision| decision.source == TuningSource::DriftRolledBack)
    }

    /// One human-readable block explaining every value that is not the
    /// operator's configured one.
    ///
    /// This is the answer to "why did this run get 5 repair iterations?". It is
    /// rendered into the run's plan output and persisted with the run metadata,
    /// so the question is answerable without opening the learning store.
    pub(crate) fn report(&self, class: &str) -> String {
        if !self.moved() {
            return String::new();
        }
        let mut out = format!("SONA-tuned generated limits (task class: {class})\n");
        for decision in &self.decisions {
            if !decision.source.moved() {
                continue;
            }
            out.push_str(&format!(
                "- {}: {} -> {} ({}, weight {:+.4}, {} observation(s))\n",
                decision.parameter.key(),
                decision.baseline,
                decision.applied,
                source_label(decision.source),
                decision.weight,
                decision.observations,
            ));
        }
        out
    }
}

fn source_label(source: TuningSource) -> &'static str {
    match source {
        TuningSource::Baseline => "configured default",
        TuningSource::InsufficientEvidence => "insufficient evidence",
        TuningSource::Learned => "learned",
        TuningSource::ClampedToFloor => "learned, held at floor",
        TuningSource::ClampedToCeiling => "learned, held at ceiling",
        TuningSource::RaisedByVerificationInvariant => {
            "raised so verification outlasts its host call"
        }
        TuningSource::DriftRolledBack => "drift detected, rolled back to default",
    }
}

/// Resolve the four generated limits for one run.
///
/// Fails closed at every step: SONA disabled, an unopenable store, an
/// unreadable route — all return the operator's configured values verbatim.
/// There is no path here that guesses.
pub(crate) fn tune_generated_config(
    project_root: &Path,
    class: &str,
    learning: &LearningConfig,
    baseline: &GeneratedWorkflowConfig,
) -> GeneratedTuning {
    if !sona_tuning_enabled(learning) {
        return GeneratedTuning::untuned(baseline.clone());
    }
    let path = learning_store_path(project_root);
    // A store that does not exist yet is the normal first-run state, not an
    // error: creating it here to read zero rows would leave a file behind that
    // implies learning happened.
    if !path.exists() {
        return GeneratedTuning::untuned(baseline.clone());
    }
    let Ok(db) = crate::command::topology_fold::open_store(&path, "learning").map_err(
        |error| tracing::debug!(%error, "learning store unavailable; generated limits untuned"),
    ) else {
        return GeneratedTuning::untuned(baseline.clone());
    };

    let observations = load_observations(&db, class);
    let tuner = SonaParameterTuner::from_history(class, &observations);
    let inputs: Vec<GeneratedTuningInput> = TunableGeneratedParameter::ALL
        .into_iter()
        .map(|parameter| {
            let tuned = tuner.weight_for(parameter.key());
            GeneratedTuningInput {
                parameter,
                weight: tuned.weight,
                observations: tuned.observations,
                drift_rolled_back: tuned.drift_rolled_back,
            }
        })
        .collect();

    if tuner.drift_rolled_back() {
        tracing::warn!(
            class,
            "SONA parameter drift exceeded the reject threshold; rolled back to the \
             configured generated limits for this run"
        );
    }

    let (config, decisions) = apply_generated_tuning(baseline, &inputs);
    GeneratedTuning { config, decisions }
}

/// Read every persisted outcome for this class, one route per parameter.
///
/// A route that will not read yields no observations for that parameter rather
/// than aborting the others: a corrupt row for one budget must not force the
/// other three back to defaults, and each parameter's evidence is independent
/// by construction.
fn load_observations(db: &cozo::DbInstance, class: &str) -> Vec<TuningObservation> {
    let mut observations = Vec::new();
    for parameter in TunableGeneratedParameter::ALL {
        let route = SonaParameterTuner::route(class, parameter.key());
        match trajectory_store::load_route_outcomes(db, &route) {
            Ok(rows) => observations.extend(rows.into_iter().map(|(pressure, recorded_at)| {
                TuningObservation {
                    parameter_key: parameter.key().to_string(),
                    pressure,
                    // Negative epochs are not representable as an ordering key
                    // and mean a clock before 1970; treat them as the oldest
                    // possible rather than wrapping into the far future.
                    recorded_at: u64::try_from(recorded_at).unwrap_or(0),
                }
            })),
            Err(error) => tracing::debug!(
                %error,
                %route,
                "tuning route unreadable; parameter holds its configured default"
            ),
        }
    }
    observations
}
