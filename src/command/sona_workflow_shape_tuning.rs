//! The read half of the SONA loop on the generated plan's *shape*.
//!
//! # What this extends, and what it deliberately does not
//!
//! Phase 7 (`sona_workflow_tuning`) tunes four scalars inside a fixed
//! topology. This module is the first knob that changes the topology itself:
//! how wide the implementation fan-out dispatches. It is a *variation within
//! the proven plan*, not a rival plan — every setting runs the same 63 stage
//! families, the same `blocked-*` terminals, the same repair loops, noop-proof
//! verification, evidence reconciliation, deadlock detection and final zero-gap
//! audit, because the knob cannot reach any of them. `sona_workflow_shape_gate`
//! proves that rather than assuming it.
//!
//! There is no motif library here and there must not be one. A rival plan would
//! have to reproduce all of the above from scratch, which is the objection that
//! deferred this work in the first place, and it still stands.
//!
//! # Keying
//!
//! One weight per `(task class, knob)`, on the same route prefix and the same
//! store as Phase 7, with a key that shares no prefix with any budget key —
//! pinned by `generated_shape_tests::shape_knobs_and_budget_parameters_share_no_route_key`.
//! Same class, same store, same evidence gate, different question.
//!
//! # What this module must never touch
//!
//! Phase 6, exactly as Phase 7 must not: nothing in `archon-knowledge` or
//! `requirement_trace*` may read a learned weight, and
//! `sona_workflow_tuning_isolation_tests.rs` fails the build if one starts
//! to. This module is additionally fenced in the other direction: it may only
//! change how work is *distributed*, never how long it may run, never how many
//! times it may retry, and never whether it is accepted.

use std::path::{Path, PathBuf};

use archon_core::config::{
    LearningConfig, ShapeDecision, ShapeInput, ShapeSource, TunableShapeKnob, decide_fanout_width,
};
use archon_pipeline::learning::sona::{SonaParameterTuner, TuningObservation};
use archon_pipeline::learning::trajectory_store;
use archon_workflow::WorkflowV2HostCall;

use crate::command::sona_workflow_shape_gate::{self, GateOutcome};
use crate::command::sona_workflow_tuning::{learning_store_path, sona_tuning_enabled};

#[path = "sona_workflow_shape_tuning_outcome.rs"]
mod sona_workflow_shape_tuning_outcome;
pub(crate) use sona_workflow_shape_tuning_outcome::record_generated_shape_outcome;

#[cfg(test)]
#[path = "sona_workflow_shape_tuning_tests.rs"]
mod tests;

/// The concurrency ceiling every fan-out in a live V2 run is clamped to.
///
/// One function so the learner and the runtime cannot disagree about what the
/// baseline *is*. `read_only_v2_fanout_parallelism` clamps whatever this module
/// proposes into `1..=this`, which is what makes the knob structurally unable
/// to create concurrency the operator did not already authorise — the property
/// the bounds module in `archon-core` relies on.
pub(crate) fn resolved_subagent_cap() -> Option<usize> {
    archon_tools::subagent_executor::get_subagent_executor()
        .and_then(|executor| executor.max_concurrency())
        .or(Some(
            archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT,
        ))
}

/// The resolved shape for one run, plus the decisions that produced it.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedShape {
    /// The width to hand the implementation fan-out, or `None` for "use the
    /// configured cap" — which is what every run got before this module
    /// existed, and what every run without proven evidence still gets.
    pub(crate) implementation_wave_width: Option<u8>,
    pub(crate) decisions: Vec<ShapeDecision>,
}

impl GeneratedShape {
    /// The untouched default, with no decisions to report.
    ///
    /// Used whenever SONA is off or the store will not open. Reporting no
    /// decisions rather than one `Baseline` decision keeps the run output
    /// silent in the overwhelmingly common case where nothing was learned.
    pub(crate) fn untuned() -> Self {
        Self {
            implementation_wave_width: None,
            decisions: Vec::new(),
        }
    }

    /// True when the operator should be told something, which includes the
    /// cases where the value did *not* change — a drift rollback and a lint
    /// refusal are both things a run kept its default because of, and both are
    /// invisible if only differences are reported.
    pub(crate) fn noteworthy(&self) -> bool {
        self.decisions
            .iter()
            .any(|decision| decision.source.noteworthy())
    }

    /// One human-readable block explaining every knob that is not at its
    /// configured default, and every proposal that was withdrawn.
    ///
    /// This is the answer to "why did this run dispatch two tasks at a time?".
    /// It is emitted before any work starts, rendered into the run's plan
    /// output, and persisted with the run metadata, so the question is
    /// answerable without opening the learning store.
    pub(crate) fn report(&self, class: &str) -> String {
        if !self.noteworthy() {
            return String::new();
        }
        let mut out = format!("SONA-tuned generated shape (task class: {class})\n");
        for decision in &self.decisions {
            if !decision.source.noteworthy() {
                continue;
            }
            out.push_str(&format!(
                "- {}: {} -> {} ({}, weight {:+.4}, {} observation(s))\n",
                decision.knob.key(),
                decision.baseline,
                decision.applied,
                source_label(decision.source),
                decision.weight,
                decision.observations,
            ));
            if let Some(refusal) = &decision.refusal {
                out.push_str(&format!("  refused before the run: {refusal}\n"));
            }
        }
        out
    }
}

fn source_label(source: ShapeSource) -> &'static str {
    match source {
        ShapeSource::Baseline => "configured default",
        ShapeSource::InsufficientEvidence => "insufficient evidence",
        ShapeSource::Learned => "learned",
        ShapeSource::ClampedToFloor => "learned, held at serial dispatch",
        ShapeSource::DriftRolledBack => "drift detected, rolled back to default",
        ShapeSource::RefusedByDependencyGraph => "proposal refused by the pre-run lint",
    }
}

/// Resolve the generated plan's shape for one run.
///
/// Fails closed at every step: SONA disabled, an unopenable store, an
/// unreadable route, a graph the lints cannot vouch for — all yield the
/// operator's configured concurrency verbatim. There is no path here that
/// guesses, and no path that widens.
pub(crate) fn tune_generated_shape(
    project_root: &Path,
    class: &str,
    learning: &LearningConfig,
    plan_calls: &[WorkflowV2HostCall],
    tasks_root: Option<&Path>,
) -> GeneratedShape {
    if !sona_tuning_enabled(learning) {
        return GeneratedShape::untuned();
    }
    let path = learning_store_path(project_root);
    // A store that does not exist yet is the normal first-run state, not an
    // error: creating it here to read zero rows would leave a file behind that
    // implies learning happened.
    if !path.exists() {
        return GeneratedShape::untuned();
    }
    let Ok(db) = crate::command::topology_fold::open_store(&path, "learning").map_err(
        |error| tracing::debug!(%error, "learning store unavailable; generated shape untuned"),
    ) else {
        return GeneratedShape::untuned();
    };

    let tuner = SonaParameterTuner::from_history(class, &load_shape_observations(&db, class));
    if tuner.drift_rolled_back() {
        tracing::warn!(
            class,
            "SONA shape drift exceeded the reject threshold; rolled back to the configured \
             concurrency for this run"
        );
    }

    let knob = TunableShapeKnob::ImplementationWaveFanoutWidth;
    let tuned = tuner.weight_for(knob.key());
    let input = ShapeInput {
        knob,
        weight: tuned.weight,
        observations: tuned.observations,
        drift_rolled_back: tuned.drift_rolled_back,
    };
    let cap = resolved_subagent_cap()
        .unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT);
    let mut decision = decide_fanout_width(u32::try_from(cap).unwrap_or(u32::MAX), Some(&input));

    if let GateOutcome::Refused(reason) =
        sona_workflow_shape_gate::admit(&mut decision, plan_calls, tasks_root)
    {
        tracing::warn!(
            class,
            %reason,
            "pre-run topology lint refused the proposed fan-out width; configured cap kept"
        );
    }

    GeneratedShape {
        // Saturating rather than `as`: the bounds module keeps every width
        // inside `1..=cap` and every realistic cap is far below 255, so this
        // cast cannot lose information for a value that module produced. A
        // future cap past 255 pins the width at 255, which is still a narrowing
        // of a wider cap and therefore still fail-closed.
        implementation_wave_width: decision
            .applied_width()
            .map(|width| u8::try_from(width).unwrap_or(u8::MAX)),
        decisions: vec![decision],
    }
}

/// Read every persisted shape outcome for this class, one route per knob.
///
/// A route that will not read yields no observations for that knob rather than
/// aborting the others, matching Phase 7: each knob's evidence is independent
/// by construction and one corrupt row must not reset the rest.
pub(crate) fn load_shape_observations(
    db: &cozo::DbInstance,
    class: &str,
) -> Vec<TuningObservation> {
    let mut observations = Vec::new();
    for knob in TunableShapeKnob::ALL {
        let route = SonaParameterTuner::route(class, knob.key());
        match trajectory_store::load_route_outcomes(db, &route) {
            Ok(rows) => observations.extend(rows.into_iter().map(|(pressure, recorded_at)| {
                TuningObservation {
                    parameter_key: knob.key().to_string(),
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
                "shape route unreadable; knob holds its configured default"
            ),
        }
    }
    observations
}

/// The directory the decomposed-PRD task files live in, for the declared
/// dependency graph the gate lints.
///
/// Derived from the task universe's own source paths rather than guessed from
/// the project root: a run whose tasks came from somewhere else must lint the
/// graph it is actually running, or the gate is scoring a different PRD.
pub(crate) fn tasks_root_of(
    universe: &crate::command::workflow_live::workflow_live_task_universe::WorkflowV2TaskUniverse,
) -> Option<PathBuf> {
    universe
        .tasks
        .iter()
        .find_map(|task| Path::new(&task.source_path).parent().map(Path::to_path_buf))
}
