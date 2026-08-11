//! Surprise-weighted replay prioritisation for world-model training (W6).
//!
//! Latent surprise has had an honest producer since `surprise_observed` landed:
//! `record_guardrail_completion_outcome` scores the world model's own
//! prediction against what the verification actually observed, and the number
//! is persisted on [`WorldGuardrailOutcome::latent_surprise`]. Nothing read it
//! back. This module is the read side — the part of W6 that turns a recorded
//! error signal into which transitions the trainer sees.
//!
//! # Why this is bounded rather than "just sample proportional to error"
//!
//! Prioritised replay changes the training distribution. That is the point, and
//! it is also the failure mode: a model trained on its own surprise drifts
//! toward whatever it was already wrong about, and an evaluation drawn from the
//! same pool moves with it, so the metric improves while the model gets worse.
//! Three independent bounds hold, each for a separate reason, and none of them
//! is a property of the surprise distribution:
//!
//! * **A uniform floor.** At most [`MAX_PRIORITIZED_FRACTION`] of any batch is
//!   drawn from the prioritised stream; the remainder is uniform over the whole
//!   training pool. This is what keeps every transition's selection probability
//!   at least `(1 - f)/n`, which in turn is what bounds the recorded importance
//!   weight above by `1/(1 - f)`. Unbounded importance weights are the usual way
//!   prioritised replay silently becomes unusable for evaluation; here they
//!   cannot exceed 2.0 at the default `f = 0.5`.
//! * **A rank-based weight cap.** A transition's weight is
//!   `1 + (C - 1) * percentile` with `C = `[`MAX_SURPRISE_WEIGHT`], where the
//!   percentile is the transition's *rank* among surprise-carrying transitions,
//!   not its surprise value. A single anomaly — a tool that returned a parse
//!   error, a run against a broken provider — produces an arbitrarily large raw
//!   surprise, and a value-proportional scheme would hand it most of the batch.
//!   Rank cannot escape `[1, C]` no matter what the value is.
//! * **A decile concentration cap.** No priority decile may supply more than
//!   [`MAX_DECILE_SHARE`] of a batch, enforced during the draw. This is exactly
//!   the roadmap's W6 automatic-rollback trigger — one priority decile
//!   supplying over 40% of a batch — and enforcing it at draw time means the
//!   monitor is
//!   watching an invariant the sampler already holds, so a breach is a bug
//!   rather than an unlucky distribution.
//!
//! Config may only tighten these. [`ReplayPolicy::clamped`] takes the operator's
//! values and clamps them into the constants above, so a hand-edited
//! `config.toml` cannot widen a bound.
//!
//! # The held-out set
//!
//! [`is_held_out`] takes a session id and the split version. It cannot see
//! surprise, replay counts, recency, labels, or model state, because they are
//! not arguments. The partition is computed *before* any weight, and the
//! prioritised pool is built only from the training side, so no weighting code
//! ever holds a held-out index.
//!
//! The split is by **session**, not by transition: adjacent transitions share
//! rows (a context window overlaps its neighbour's target), so a per-transition
//! split would put a training transition's target inside a held-out context and
//! call the leak an evaluation.
//!
//! Hashing is FNV-1a rather than [`std::collections::hash_map::DefaultHasher`],
//! whose output is explicitly not stable across releases — a corpus's partition
//! must not silently re-shuffle when the toolchain moves, or every historical
//! evaluation window becomes unreproducible.
//!
//! # Default: computed always, applied only when enabled
//!
//! [`ReplayPolicy::prioritized_enabled`] defaults to `false`. The plan is still
//! built on every training run and reported, so the sampler is exercised on live
//! corpora and its concentration, coverage and importance-weight range are
//! visible before anything depends on them. Only [`ReplayPlan::applied`] decides
//! whether the trainer actually narrows its example set. That is the
//! shadow-before-canary shape the roadmap requires of any behaviour-changing
//! slice, and it means enabling the flag is the only thing that changes what the
//! model learns from.
//!
//! [`WorldGuardrailOutcome::latent_surprise`]: crate::WorldGuardrailOutcome

#[path = "replay/00_policy.rs"]
mod policy;
#[path = "replay/01_sampler.rs"]
mod sampler;
#[cfg(test)]
#[path = "replay/02_tests.rs"]
mod tests;

pub use policy::{
    DEFAULT_HELD_OUT_FRACTION, HELD_OUT_SPLIT_SALT, MAX_DECILE_SHARE, MAX_PRIORITIZED_FRACTION,
    MAX_SURPRISE_WEIGHT, PRIORITY_VERSION, ReplayPolicy, TransitionKey, is_held_out,
    surprise_by_row_id,
};
pub use sampler::{ReplayPlan, ReplaySample, ReplaySkipReason, ReplaySummary, plan_replay};
