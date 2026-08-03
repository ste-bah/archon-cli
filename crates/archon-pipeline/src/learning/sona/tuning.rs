//! Turn recorded workflow outcomes into a SONA weight per tunable parameter.
//!
//! # What the weight means here
//!
//! SONA's `quality` is a free scalar; this module fixes its meaning for the
//! tuning routes and nothing else reuses it. On a tuning route,
//! **`quality` is budget pressure**: the fraction of a budget the observed run
//! actually needed. `1.0` means the run exhausted the budget, `0.0` means it
//! did not touch it, `0.5` means it used exactly half and is the neutral point
//! (`calculate_gradient` subtracts 0.5, so a pressure of 0.5 moves nothing).
//!
//! That gives the weight a direction with no extra machinery: a positive weight
//! means observed runs kept running out of this budget, so it should grow; a
//! negative weight means observed runs finished well inside it, so it may
//! shrink. [`archon_core::config::apply_generated_tuning`] is what turns that
//! into a value, and it owns the bounds.
//!
//! Naming this explicitly matters because the obvious reading is wrong: a run
//! that hit a timeout is a *bad* run with a *high* pressure. Anyone who reads
//! `quality: 1.0` on a tuning row as "this run went well" will invert the sign
//! of the whole loop.
//!
//! # Why the engine is rebuilt from persisted rows instead of kept alive
//!
//! `SonaEngine` holds weights in memory and a CLI process lives for one run.
//! The durable artefact is the `trajectories` relation, which already persists
//! one row per observation. So the tuner replays those rows through the real
//! engine every time: the weights are a pure function of the recorded evidence,
//! which is also what makes the fail-closed rule checkable — delete the rows and
//! the weights are gone, not merely hidden.

use std::collections::HashMap;

use super::config::SonaConfig;
use super::constants::DEFAULT_DRIFT_REJECT_THRESHOLD;
use super::engine::SonaEngine;
use super::types::{DriftReport, DriftStatus, FeedbackInput};

/// Learning rate for tuning routes only.
///
/// SONA's 0.01 default is calibrated for one trajectory per agent step —
/// thousands per run. A tuning route sees exactly one observation per
/// *completed workflow run*, so at 0.01 a consistently-pressured budget needs
/// well over a hundred runs to move a single integer step, which is longer than
/// most projects last: the loop would be closed on paper and open in practice.
/// At 0.05 it takes roughly fourteen consistent observations, which is
/// comfortably above [`MIN_OBSERVATIONS`] — the value can only start moving
/// once the evidence gate has already been cleared with room to spare.
pub const TUNING_LEARNING_RATE: f64 = 0.05;

/// Observations required on a key before its weight may be used at all.
///
/// Below five, one anomalous run is at least a fifth of the entire signal, and
/// a budget moved by one bad afternoon is worse than a static default because
/// it looks principled. Under the threshold the tuner reports no weight, and no
/// weight means the operator's configured value.
pub const MIN_OBSERVATIONS: u32 = 5;

/// SONA writes weights under a fixed pattern id per route; tuning routes are
/// one weight each, so they use the same id `provide_feedback` writes.
const TUNING_PATTERN_ID: &str = "default";

/// The route prefix that separates tuning weights from agent trajectories.
///
/// Kept distinct so a tuning row can never be mistaken for an agent outcome by
/// the GNN trainer, which trains on trajectory embeddings and would be learning
/// from rows that describe a config value rather than a piece of work.
pub const TUNING_ROUTE_PREFIX: &str = "tuning/generated";

/// One recorded outcome for one parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct TuningObservation {
    /// Parameter key, matching `TunableGeneratedParameter::key`.
    pub parameter_key: String,
    /// Budget pressure in `0.0..=1.0`. See the module docs — this is *not* run
    /// quality, and a value outside the range is clamped rather than trusted.
    pub pressure: f64,
    /// Epoch seconds. Replay order is by this, so a clock that goes backwards
    /// reorders history rather than corrupting it.
    pub recorded_at: u64,
}

/// What happened when a candidate observation batch was offered to the tuner.
#[derive(Debug, Clone)]
pub enum AdmissionOutcome {
    /// The batch moved the weights within the configured drift tolerance.
    Admitted(DriftReport),
    /// The batch diverged past the reject threshold. The tuner has rolled back
    /// to its checkpoint and the caller must not persist the batch.
    DriftRejected(DriftReport),
}

/// Weight lookup plus the evidence behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TunedWeight {
    /// `None` when the key has fewer than [`MIN_OBSERVATIONS`] outcomes, or
    /// when drift forced a rollback. Both mean: use the configured default.
    pub weight: Option<f64>,
    pub observations: u32,
    pub drift_rolled_back: bool,
}

/// Replays recorded outcomes through a real [`SonaEngine`] and reports the
/// resulting weight per parameter.
pub struct SonaParameterTuner {
    engine: SonaEngine,
    /// Task class the routes are keyed on. One tuner instance is one class.
    class: String,
    /// Observations replayed per parameter key. The engine cannot answer this:
    /// `get_weight` returns 0.0 both for "no evidence" and for "evidence that
    /// says do not move", and those must never be conflated.
    counts: HashMap<String, u32>,
    /// Set when [`Self::from_history`] found the newest observation diverging
    /// past the reject threshold and rolled back.
    drift_rolled_back: bool,
}

impl SonaParameterTuner {
    /// The route a parameter's weight lives on.
    ///
    /// Task class is part of the key because the budgets differ by kind of
    /// work, not by project: a bug hunt's repair loop converges or does not,
    /// while a greenfield build rarely enters it at all, and averaging the two
    /// produces a number that describes neither. Both halves are stable strings
    /// (`TaskClass::as_str` and `TunableGeneratedParameter::key`) so a stored
    /// weight keeps its meaning across releases.
    #[must_use]
    pub fn route(class: &str, parameter_key: &str) -> String {
        format!("{TUNING_ROUTE_PREFIX}/{class}/{parameter_key}")
    }

    /// Build a tuner by replaying persisted observations.
    ///
    /// The newest observation is applied *after* a checkpoint so drift can be
    /// evaluated against the model the rest of history built. If it diverges
    /// past the reject threshold the tuner rolls back and reports
    /// `drift_rolled_back`, which resolves to the configured default for every
    /// key: a regime the accumulated evidence does not explain is exactly the
    /// case where a learned number should not be trusted.
    #[must_use]
    pub fn from_history(class: &str, observations: &[TuningObservation]) -> Self {
        let mut ordered: Vec<&TuningObservation> = observations.iter().collect();
        ordered.sort_by_key(|observation| observation.recorded_at);

        let mut tuner = Self {
            engine: SonaEngine::new(SonaConfig {
                learning_rate: TUNING_LEARNING_RATE,
                // No DB and no embedding provider: replay must not write rows,
                // or reading history would grow it.
                ..SonaConfig::default()
            }),
            class: class.to_string(),
            counts: HashMap::new(),
            drift_rolled_back: false,
        };

        let split = ordered.len().saturating_sub(1);
        for observation in &ordered[..split] {
            tuner.apply(observation);
        }
        let Some(newest) = ordered.last() else {
            return tuner;
        };

        let keys = tuner.tracked_keys(&ordered);
        let before = tuner.weight_vector(&keys);
        tuner.engine.save_checkpoint();
        tuner.apply(newest);
        let after = tuner.weight_vector(&keys);
        if tuner.drift(&before, &after).status == DriftStatus::Reject {
            tuner.engine.rollback();
            tuner.drift_rolled_back = true;
        }
        tuner
    }

    /// The weight for one parameter, or `None` when the evidence is too thin.
    #[must_use]
    pub fn weight_for(&self, parameter_key: &str) -> TunedWeight {
        let observations = self.counts.get(parameter_key).copied().unwrap_or(0);
        let proven = observations >= MIN_OBSERVATIONS && !self.drift_rolled_back;
        TunedWeight {
            weight: proven.then(|| {
                self.engine
                    .get_weight(&Self::route(&self.class, parameter_key), TUNING_PATTERN_ID)
            }),
            observations,
            drift_rolled_back: self.drift_rolled_back,
        }
    }

    /// True when [`Self::from_history`] rolled back on drift.
    #[must_use]
    pub fn drift_rolled_back(&self) -> bool {
        self.drift_rolled_back
    }

    /// Offer a new batch of observations and decide whether it may be persisted.
    ///
    /// This is the write-side gate. The tuner checkpoints, applies the batch,
    /// and compares the weight vector before and after; a batch that diverges
    /// past the reject threshold is rolled back and must not reach the store.
    /// Without this a single pathological run — one where every branch timed
    /// out because a laptop went to sleep — permanently moves every budget it
    /// touched, and nothing would ever move it back.
    pub fn admit(&mut self, candidates: &[TuningObservation]) -> AdmissionOutcome {
        let keys = self.tracked_keys(&candidates.iter().collect::<Vec<_>>());
        let before = self.weight_vector(&keys);
        self.engine.save_checkpoint();
        for candidate in candidates {
            self.apply(candidate);
        }
        let after = self.weight_vector(&keys);
        let report = self.drift(&before, &after);
        if report.status == DriftStatus::Reject {
            self.engine.rollback();
            for candidate in candidates {
                if let Some(count) = self.counts.get_mut(&candidate.parameter_key) {
                    *count = count.saturating_sub(1);
                }
            }
            return AdmissionOutcome::DriftRejected(report);
        }
        AdmissionOutcome::Admitted(report)
    }

    /// Feed one observation through the real engine.
    ///
    /// `l_score` and `success_rate` are held at 1.0 so `calculate_reward`
    /// reduces to the pressure itself. Anything else would make the recorded
    /// row and the replayed weight disagree, and the row is the only durable
    /// half.
    fn apply(&mut self, observation: &TuningObservation) {
        let route = Self::route(&self.class, &observation.parameter_key);
        let trajectory =
            self.engine
                .create_trajectory(&route, "generated-workflow-tuner", "tuning");
        let _ = self.engine.provide_feedback(&FeedbackInput {
            trajectory_id: trajectory.trajectory_id,
            quality: observation.pressure.clamp(0.0, 1.0),
            l_score: 1.0,
            success_rate: 1.0,
        });
        *self
            .counts
            .entry(observation.parameter_key.clone())
            .or_insert(0) += 1;
    }

    /// Every key this tuner has seen, plus the ones in `extra`, sorted.
    ///
    /// Sorted so the drift vector's coordinates are stable across calls; an
    /// unstable order makes cosine similarity compare unrelated parameters.
    fn tracked_keys(&self, extra: &[&TuningObservation]) -> Vec<String> {
        let mut keys: Vec<String> = self.counts.keys().cloned().collect();
        for observation in extra {
            if !keys.contains(&observation.parameter_key) {
                keys.push(observation.parameter_key.clone());
            }
        }
        keys.sort();
        keys
    }

    /// Drift between two weight vectors, with the empty-prior case excluded.
    ///
    /// `cosine_similarity` returns 0.0 against a zero-norm vector, which
    /// `check_drift` reads as total divergence. A key with no prior weight
    /// therefore rejects its own first observations forever and the loop never
    /// starts — the fail-closed rule would have eaten the learner. There is
    /// genuinely nothing to drift *from* before the first weight exists, so the
    /// honest report for that case is Normal.
    fn drift(&self, before: &[f64], after: &[f64]) -> DriftReport {
        let prior_norm: f64 = before.iter().map(|value| value * value).sum::<f64>().sqrt();
        if prior_norm < 1e-12 {
            return DriftReport {
                status: DriftStatus::Normal,
                divergence: 0.0,
                threshold_used: DEFAULT_DRIFT_REJECT_THRESHOLD,
            };
        }
        self.engine.check_drift(before, after)
    }

    fn weight_vector(&self, keys: &[String]) -> Vec<f64> {
        keys.iter()
            .map(|key| {
                self.engine
                    .get_weight(&Self::route(&self.class, key), TUNING_PATTERN_ID)
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "tuning_tests.rs"]
mod tests;
