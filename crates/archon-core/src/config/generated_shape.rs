//! Safety bounds for the learned *shape* of a generated run.
//!
//! # What separates this from [`super::generated_tuning`]
//!
//! Phase 7 tunes four scalars that say how long something may run and how many
//! times a loop may retry. Nothing it can do changes which stages exist or how
//! work is distributed across branches. This module tunes a *structural* knob —
//! how wide the implementation fan-out dispatches — and structure is where the
//! expensive mistakes live, so the rules here are strictly tighter than Phase
//! 7's, not merely analogous.
//!
//! The two knob sets are deliberately disjoint types with disjoint route keys.
//! A reviewer must be able to read `TunableGeneratedParameter` and conclude
//! "this can only move timeouts and retry counts" without also having to know
//! what this module does. `generated_shape_tests` pins that disjointness.
//!
//! # The one knob, and why width is the one that is safe to learn
//!
//! `implementation_wave_fanout_width` is the number of ready tasks the write
//! fan-out dispatches concurrently.
//!
//! It is the only structural knob in this module because it is the only one
//! whose dangerous direction is already closed by shipped code:
//! `read_only_v2_fanout_parallelism` clamps any requested width into
//! `1..=subagent_cap`, so **no value this module can produce is able to create
//! concurrency the operator did not already authorise**. The learner can only
//! ever ask for *less* than the configured cap. That property is what makes a
//! structural knob admissible at all — the review-granularity and
//! verifier-count knobs have no such clamp behind them, and a learner that got
//! those wrong would remove a reviewer rather than slow a wave down.
//!
//! # The four rules every shape value obeys
//!
//! 1. **No evidence, no move.** [`ShapeInput::weight`] is `None` until the
//!    learner has met its own evidence threshold, and a `None` weight yields
//!    the configured baseline unchanged. Byte-identical to today's behaviour.
//! 2. **Narrowing only.** A weight can lower the width and can never raise it.
//!    See [`decide_fanout_width`] for the sign convention, which is *inverted*
//!    relative to Phase 7 and is the single easiest thing to get wrong here.
//! 3. **Bounds bind over the learned value**, and clamping is recorded in the
//!    decision rather than applied silently.
//! 4. **The graph gets a veto, not a vote.** This module produces a *proposal*.
//!    The caller must put it through the topology lints before it reaches a run
//!    and record the outcome with [`ShapeDecision::refuse`]. A proposal that
//!    the declared dependency graph cannot support is not applied.

use serde::{Deserialize, Serialize};

/// How far a fully saturated weight may narrow the width, as a fraction of the
/// baseline cap.
///
/// At `weight = 1.0` — SONA's clamp limit — the width halves. Deliberately the
/// same span Phase 7 uses: a structural knob that could move further per run
/// than a timeout knob would reach its floor before an operator noticed it had
/// started moving at all.
pub const SHAPE_TUNING_SPAN: f64 = 0.5;

/// The structural knobs a learner may propose a value for.
///
/// One variant today. It is an enum rather than a bare constant because the
/// route key, the floor, and the report label all have to stay attached to the
/// same thing, and because a second knob must be forced to state its own floor
/// and its own justification rather than inheriting this one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunableShapeKnob {
    ImplementationWaveFanoutWidth,
}

impl TunableShapeKnob {
    /// Every knob, in report order. Also the coordinate order of the weight
    /// vector the learner runs drift detection over, so it must stay stable.
    pub const ALL: [Self; 1] = [Self::ImplementationWaveFanoutWidth];

    /// Stable key, used verbatim in learner route strings. Changing one orphans
    /// every weight already recorded against it.
    ///
    /// It shares no prefix with any [`super::TunableGeneratedParameter::key`],
    /// so a shape weight and a budget weight can never collide on a route.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::ImplementationWaveFanoutWidth => "implementation_wave_fanout_width",
        }
    }

    /// Narrowest width a learner may reach, and what breaks below it.
    #[must_use]
    pub fn floor(self) -> u32 {
        match self {
            // 1. A wave that dispatches zero branches completes zero tasks, and
            // a lifecycle whose waves complete nothing runs until
            // `max_dependency_waves` and terminates at
            // `blocked-loop-exhaustion`. That is the earned failure handling
            // firing on a defect the knob invented, which is worse than no knob
            // at all: the run reports the shape of a genuinely stuck plan.
            // Serial dispatch is slow; zero dispatch is a manufactured deadlock.
            Self::ImplementationWaveFanoutWidth => 1,
        }
    }
}

/// Where a knob's final value came from. Serialized into the run's metadata so
/// "why did this run dispatch two tasks at a time?" is answerable from the run
/// directory alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeSource {
    /// No learner ran, or the learner is disabled. The configured cap, verbatim.
    Baseline,
    /// A learner ran but the key had too little proven evidence to move.
    InsufficientEvidence,
    /// A learned weight was applied and landed strictly inside the bounds.
    Learned,
    /// A learned weight would have gone below the floor.
    ClampedToFloor,
    /// The learner detected drift against its own checkpoint and rolled back.
    DriftRolledBack,
    /// A learned narrowing was proposed and the declared dependency graph could
    /// not support the claim, so the baseline was kept. Carries a reason in
    /// [`ShapeDecision::refusal`].
    RefusedByDependencyGraph,
}

impl ShapeSource {
    /// True when the run did not get the configured value.
    #[must_use]
    pub fn moved(self) -> bool {
        matches!(self, Self::Learned | Self::ClampedToFloor)
    }

    /// True when something happened that the operator should be told about even
    /// though the value did not change.
    #[must_use]
    pub fn noteworthy(self) -> bool {
        self.moved() || matches!(self, Self::DriftRolledBack | Self::RefusedByDependencyGraph)
    }
}

/// A learner's proposal for one structural knob.
#[derive(Debug, Clone, Copy)]
pub struct ShapeInput {
    pub knob: TunableShapeKnob,
    /// `None` means "no proven evidence" and is the only honest value when the
    /// learner has not met its own threshold. Not the same as `Some(0.0)`,
    /// which means "evidence exists and says: do not move".
    pub weight: Option<f64>,
    /// How many recorded outcomes back the weight. Reported, never used to
    /// scale the value.
    pub observations: u32,
    /// Set when the learner rolled back on drift, so the report says so rather
    /// than reporting an indistinguishable "no evidence".
    pub drift_rolled_back: bool,
}

/// What one knob ended up at, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeDecision {
    pub knob: TunableShapeKnob,
    /// The operator's effective value — for width, the resolved subagent cap.
    pub baseline: u32,
    pub applied: u32,
    pub weight: f64,
    pub observations: u32,
    pub source: ShapeSource,
    /// Present only on [`ShapeSource::RefusedByDependencyGraph`]: which lint
    /// refused, and what it found. Empty on every other path so the common
    /// case costs nothing in the persisted metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

impl ShapeDecision {
    /// Withdraw a proposed narrowing and hold the baseline.
    ///
    /// Called by the pre-run lint gate. The proposal is discarded rather than
    /// partially honoured: a width justified by a graph the lints could not
    /// vouch for is a number with no argument behind it, and the operator's
    /// configured cap at least has an author.
    pub fn refuse(&mut self, reason: impl Into<String>) {
        self.applied = self.baseline;
        self.source = ShapeSource::RefusedByDependencyGraph;
        self.refusal = Some(reason.into());
    }

    /// Narrow further than the learner asked, because the graph says the extra
    /// width is unreachable.
    ///
    /// Only ever tightens: a `limit` at or above the current value is ignored,
    /// so a lint can never be the reason a run got *more* concurrency.
    pub fn tighten_to(&mut self, limit: u32) {
        let limit = limit.max(self.knob.floor());
        if limit >= self.applied {
            return;
        }
        self.applied = limit;
    }

    /// The width to hand the run, or `None` for "use the configured cap".
    ///
    /// `None` rather than `Some(baseline)` on the unmoved path so the run's
    /// options stay byte-identical to a run where this module never existed.
    #[must_use]
    pub fn applied_width(&self) -> Option<u32> {
        (self.applied < self.baseline).then_some(self.applied)
    }
}

/// Decide the implementation-wave fan-out width for one run.
///
/// # The sign convention, which is inverted relative to Phase 7
///
/// On a Phase 7 budget route a positive weight means "runs kept exhausting this
/// budget, grow it". On this route a positive weight means "runs kept hitting
/// contention at this width, **shrink** it". The quantity being learned is
/// pressure in both cases; what pressure implies about the value is opposite,
/// because a budget that is too small stalls a run while a fan-out that is too
/// wide corrupts one.
///
/// A negative weight — evidence that runs were never contended — is
/// deliberately inert. It is not evidence that a wider fan-out would have been
/// fine, because the width was already at the operator's cap and no run ever
/// tried to exceed it; concluding "widen" from it would be concluding something
/// no run ever tested. This is the same ratchet argument Phase 7 applies to its
/// timeout parameters, in the mirrored direction.
///
/// `baseline_cap` is the operator's resolved concurrency cap. Pure and total:
/// any input produces a width in `1..=baseline_cap`.
#[must_use]
pub fn decide_fanout_width(baseline_cap: u32, input: Option<&ShapeInput>) -> ShapeDecision {
    let knob = TunableShapeKnob::ImplementationWaveFanoutWidth;
    // A zero or absent cap is not a width this module can reason about, and
    // inventing one would be inventing concurrency. Treat it as 1.
    let baseline = baseline_cap.max(knob.floor());
    let mut decision = ShapeDecision {
        knob,
        baseline,
        applied: baseline,
        weight: 0.0,
        observations: input.map_or(0, |input| input.observations),
        source: ShapeSource::Baseline,
        refusal: None,
    };

    let Some(input) = input else {
        return decision;
    };
    if input.drift_rolled_back {
        decision.source = ShapeSource::DriftRolledBack;
        return decision;
    }
    let Some(weight) = input.weight else {
        decision.source = ShapeSource::InsufficientEvidence;
        return decision;
    };
    // A non-finite weight is a learner bug, and the fail-closed answer to a
    // learner bug is the operator's configured value.
    if !weight.is_finite() {
        decision.source = ShapeSource::InsufficientEvidence;
        return decision;
    }

    decision.weight = weight;
    // Clamped at 0.0 on the low side, not -1.0: see the ratchet argument above.
    // Negative pressure reaches this line as 0.0 and moves nothing, which is
    // reported as `Learned` with `applied == baseline` — evidence that was
    // consulted and said "hold" is not the same as evidence that was missing.
    let narrowing = weight.clamp(0.0, 1.0) * SHAPE_TUNING_SPAN;
    let proposed = f64::from(baseline) * (1.0 - narrowing);
    // `round` then saturate: the product of a u32 baseline and at most 1.0
    // cannot exceed f64 precision, and the clamp below is what bounds it, so
    // the cast only has to be non-panicking.
    let rounded = proposed.round().clamp(0.0, f64::from(u32::MAX)) as u32;
    let floor = knob.floor();

    decision.applied = rounded.clamp(floor, baseline);
    decision.source = if rounded < floor {
        ShapeSource::ClampedToFloor
    } else {
        ShapeSource::Learned
    };
    decision
}

#[cfg(test)]
#[path = "generated_shape_tests.rs"]
mod tests;
