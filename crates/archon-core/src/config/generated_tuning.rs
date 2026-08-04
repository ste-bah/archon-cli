//! Safety bounds for learned [`GeneratedWorkflowConfig`] values.
//!
//! # Why the bounds live here and not in the learner
//!
//! SONA produces a weight. A weight is a number with no idea what it is
//! measuring. The knowledge that a verifier which gets less wall-clock than the
//! work it inspects will VOID correct work rather than fail honestly is a fact
//! about *this config*, not about gradient descent — so it is stated next to
//! the config, in a module the learner depends on rather than one that depends
//! on the learner. `archon-core` has no learning dependency and must not gain
//! one: that keeps the bounds authoritative. A learner that wanted to widen a
//! bound would have to edit this file, in a diff a reviewer reads.
//!
//! # The three rules every tuned value obeys
//!
//! 1. **No evidence, no move.** [`GeneratedTuningInput::weight`] is `None`
//!    whenever the learner has not accumulated proven outcomes for a key, and a
//!    `None` weight yields the configured baseline unchanged. There is no
//!    exploration term. An unproven weight is not a weight.
//! 2. **Bounds bind over the learned value.** Every parameter has a floor and a
//!    ceiling justified below by what breaks when it is crossed. Clamping is
//!    recorded in the decision, not silent.
//! 3. **The verification invariant is checked last.** Per-parameter clamping
//!    cannot express "verification must outlast the work it verifies", so that
//!    is enforced across parameters after both are clamped.

use super::sections::GeneratedWorkflowConfig;
use serde::{Deserialize, Serialize};

/// How far a fully saturated weight may move a value, as a fraction of the
/// configured baseline.
///
/// At `weight = ±1.0` — SONA's clamp limits — a value moves at most half its
/// baseline. The span is deliberately smaller than the distance to most bounds
/// so that the bounds are a backstop rather than the normal operating point: a
/// tuner sitting permanently on a clamp is indistinguishable from a tuner with
/// a sign error, and this module wants those to look different.
pub const TUNING_SPAN: f64 = 0.5;

/// The four `[workflow.generated]` values a learner may propose a value for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunableGeneratedParameter {
    MaxRepairIterations,
    MaxInvestigationIterations,
    VerificationBranchTimeoutSecs,
    HostCallTimeoutSecs,
}

/// Where a parameter's final value came from. Serialized into the run's
/// metadata so "why did this run get 5 repair iterations?" is answerable from
/// the run directory alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningSource {
    /// No learner ran, or the learner is disabled. Baseline verbatim.
    Baseline,
    /// A learner ran but the key had too little proven evidence to move.
    InsufficientEvidence,
    /// A learned weight was applied and landed strictly inside the bounds.
    Learned,
    /// A learned weight would have gone below the floor.
    ClampedToFloor,
    /// A learned weight would have gone above the ceiling.
    ClampedToCeiling,
    /// Raised so verification outlasts the host call it verifies.
    RaisedByVerificationInvariant,
    /// The learner detected drift against its own checkpoint and rolled back,
    /// so the baseline is used for this run.
    DriftRolledBack,
}

impl TuningSource {
    /// True when the run did not get the configured value.
    #[must_use]
    pub fn moved(self) -> bool {
        !matches!(self, Self::Baseline | Self::InsufficientEvidence)
    }
}

/// A learner's proposal for one parameter.
#[derive(Debug, Clone, Copy)]
pub struct GeneratedTuningInput {
    pub parameter: TunableGeneratedParameter,
    /// `None` means "no proven evidence" and is the only honest value when the
    /// learner has not met its own evidence threshold. It is not the same as
    /// `Some(0.0)`, which means "evidence exists and says: do not move".
    pub weight: Option<f64>,
    /// How many recorded outcomes back the weight. Reported, never used to
    /// scale the value — a bigger `n` makes a weight trustworthy, not bigger.
    pub observations: u32,
    /// Set when the learner rolled back on drift, so the report says so rather
    /// than reporting an indistinguishable "no evidence".
    pub drift_rolled_back: bool,
}

/// What one parameter ended up at, and why.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeneratedTuningDecision {
    pub parameter: TunableGeneratedParameter,
    pub baseline: u32,
    pub applied: u32,
    pub weight: f64,
    pub observations: u32,
    pub source: TuningSource,
}

impl TunableGeneratedParameter {
    /// Every parameter, in report order. Also the coordinate order of the
    /// weight vector the learner runs drift detection over, so it must stay
    /// stable — reordering silently reinterprets stored weights.
    pub const ALL: [Self; 4] = [
        Self::MaxRepairIterations,
        Self::MaxInvestigationIterations,
        Self::VerificationBranchTimeoutSecs,
        Self::HostCallTimeoutSecs,
    ];

    /// Stable key, identical to the TOML field name. Used verbatim in learner
    /// route strings, so changing one orphans every weight already recorded.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::MaxRepairIterations => "max_repair_iterations",
            Self::MaxInvestigationIterations => "max_investigation_iterations",
            Self::VerificationBranchTimeoutSecs => "verification_branch_timeout_secs",
            Self::HostCallTimeoutSecs => "host_call_timeout_secs",
        }
    }

    /// The configured value this parameter is tuned relative to.
    #[must_use]
    pub fn baseline(self, config: &GeneratedWorkflowConfig) -> u32 {
        match self {
            Self::MaxRepairIterations => u32::from(config.max_repair_iterations),
            Self::MaxInvestigationIterations => u32::from(config.max_investigation_iterations),
            Self::VerificationBranchTimeoutSecs => config.verification_branch_timeout_secs,
            Self::HostCallTimeoutSecs => config.host_call_timeout_secs,
        }
    }

    /// Lowest value a learner may reach. What breaks below it, per parameter:
    #[must_use]
    pub fn floor(self) -> u32 {
        match self {
            // At 1 there is exactly one remediation attempt, so a single
            // transient — a flaky test, a provider 429, a worktree lock lost to
            // a sibling — terminates the run as unresolved with no second look.
            // Distinguishing transient from real is the entire purpose of the
            // repair loop and one attempt cannot do it.
            Self::MaxRepairIterations => 2,
            // Investigation is the read-only twin of repair and fails the same
            // way: one pass cannot separate "the evidence is genuinely absent"
            // from "the first search missed it", and the run then reports a gap
            // that does not exist.
            Self::MaxInvestigationIterations => 2,
            // 2 hours, equal to the DEFAULT host_call_timeout_secs. Observed
            // live: at 1200s a verifier timed out and VOIDED an already-accepted
            // remediation, recording correct work as unresolved. A verifier that
            // cannot finish does not fail closed honestly — it disappears, and
            // its silence is read as a failure of the work rather than of the
            // verifier. Below this floor that incident is reachable again, with
            // a learned number instead of a hardcoded one. The floor is only
            // half the protection; see [`enforce_verification_invariant`].
            Self::VerificationBranchTimeoutSecs => 7_200,
            // 30 minutes. A coder branch doing a real multi-file change with
            // tests routinely runs past 20 minutes; below 30 the timeout starts
            // killing work that was progressing, and killed work is recorded as
            // a failure that the repair loop then re-attempts. The learner would
            // be manufacturing the very failures it learns from.
            Self::HostCallTimeoutSecs => 1_800,
        }
    }

    /// Highest value a learner may reach. What breaks above it, per parameter:
    #[must_use]
    pub fn ceiling(self) -> u32 {
        match self {
            // Config validation permits 8. The learner stops at 6 so that a
            // value sitting at the schema limit still means "an operator wrote
            // that", not "the learner walked to the edge" — those must not be
            // indistinguishable. Past 6 a genuinely stuck task is re-running the
            // same failing remediation on someone's budget; the honest terminal
            // state is blocked, not another attempt.
            Self::MaxRepairIterations => 6,
            Self::MaxInvestigationIterations => 6,
            // 8 hours, twice the default. Validation permits 24, but a branch
            // allowed a full day is a hung verifier that nobody can tell from a
            // slow one, holding its worktree for the whole time while the run
            // reports nothing.
            Self::VerificationBranchTimeoutSecs => 28_800,
            // 4 hours, twice the default and equal to the verification
            // baseline. Deliberately not higher: the invariant that verification
            // outlasts the work it inspects is cheap to hold while host calls
            // stay under the verification baseline, and expensive to reason
            // about once they can exceed it.
            Self::HostCallTimeoutSecs => 14_400,
        }
    }

    fn write_into(self, config: &mut GeneratedWorkflowConfig, value: u32) {
        match self {
            // Saturating: the ceilings above are all <= u8::MAX, so this cast
            // cannot lose information for a value this module produced. It is
            // saturating rather than `as` so that a future ceiling raised past
            // 255 fails loud at the bound instead of wrapping to a small number.
            Self::MaxRepairIterations => {
                config.max_repair_iterations = u8::try_from(value).unwrap_or(u8::MAX);
            }
            Self::MaxInvestigationIterations => {
                config.max_investigation_iterations = u8::try_from(value).unwrap_or(u8::MAX);
            }
            Self::VerificationBranchTimeoutSecs => {
                config.verification_branch_timeout_secs = value;
            }
            Self::HostCallTimeoutSecs => config.host_call_timeout_secs = value,
        }
    }
}

/// Apply learner proposals to a baseline config.
///
/// Pure and total: any input produces a config that still satisfies
/// [`super::validation`], because every bound in this module is a strict subset
/// of the validated range. Callers get the decisions back so the run can report
/// them; a caller that drops them has made the tuning invisible, which
/// [`TuningSource`] exists to prevent.
#[must_use]
pub fn apply_generated_tuning(
    baseline: &GeneratedWorkflowConfig,
    inputs: &[GeneratedTuningInput],
) -> (GeneratedWorkflowConfig, Vec<GeneratedTuningDecision>) {
    let mut tuned = baseline.clone();
    let mut decisions = Vec::with_capacity(TunableGeneratedParameter::ALL.len());

    for parameter in TunableGeneratedParameter::ALL {
        let input = inputs.iter().find(|input| input.parameter == parameter);
        let decision = decide(parameter, baseline, input);
        parameter.write_into(&mut tuned, decision.applied);
        decisions.push(decision);
    }

    enforce_verification_invariant(&mut tuned, &mut decisions);
    (tuned, decisions)
}

fn decide(
    parameter: TunableGeneratedParameter,
    baseline_config: &GeneratedWorkflowConfig,
    input: Option<&GeneratedTuningInput>,
) -> GeneratedTuningDecision {
    let baseline = parameter.baseline(baseline_config);
    let mut decision = GeneratedTuningDecision {
        parameter,
        baseline,
        applied: baseline,
        weight: 0.0,
        observations: input.map_or(0, |input| input.observations),
        source: TuningSource::Baseline,
    };

    let Some(input) = input else {
        return decision;
    };
    if input.drift_rolled_back {
        decision.source = TuningSource::DriftRolledBack;
        return decision;
    }
    let Some(weight) = input.weight else {
        decision.source = TuningSource::InsufficientEvidence;
        return decision;
    };
    // A non-finite weight is a learner bug, and the fail-closed answer to a
    // learner bug is the operator's configured value.
    if !weight.is_finite() {
        decision.source = TuningSource::InsufficientEvidence;
        return decision;
    }

    decision.weight = weight;
    let proposed = f64::from(baseline) * (1.0 + weight.clamp(-1.0, 1.0) * TUNING_SPAN);
    // `round` then saturate: the product of a u32 baseline and at most 1.5
    // cannot exceed f64 precision, and the clamp below is what actually bounds
    // it, so the cast only has to be non-panicking.
    let rounded = proposed.round().clamp(0.0, f64::from(u32::MAX)) as u32;
    let (floor, ceiling) = (parameter.floor(), parameter.ceiling());

    decision.applied = rounded.clamp(floor, ceiling);
    decision.source = if rounded < floor {
        TuningSource::ClampedToFloor
    } else if rounded > ceiling {
        TuningSource::ClampedToCeiling
    } else {
        TuningSource::Learned
    };
    decision
}

/// Verification must outlast the host call it verifies.
///
/// The per-parameter floors cannot express this: `host_call_timeout_secs` may
/// legitimately be learned up to 4 hours while `verification_branch_timeout_secs`
/// may legitimately be learned down to 2, and each is individually inside its
/// bounds while the pair reproduces the observed incident — a verifier that runs
/// out of clock mid-inspection and takes an accepted remediation down with it.
///
/// Verification is raised rather than the host call lowered: lowering the host
/// call kills work that was progressing, which manufactures failures. Raising
/// verification only costs wall-clock on a branch that was going to be voided.
///
/// Only applied when learning actually moved something. An operator whose
/// configured pair already violates this has made a decision this module was
/// not asked to overrule, and silently editing it would break the rule that a
/// key with no evidence keeps its configured value.
fn enforce_verification_invariant(
    tuned: &mut GeneratedWorkflowConfig,
    decisions: &mut [GeneratedTuningDecision],
) {
    if !decisions.iter().any(|decision| decision.source.moved()) {
        return;
    }
    if tuned.verification_branch_timeout_secs >= tuned.host_call_timeout_secs {
        return;
    }
    let required = tuned.host_call_timeout_secs;
    tuned.verification_branch_timeout_secs = required;
    if let Some(decision) = decisions
        .iter_mut()
        .find(|d| d.parameter == TunableGeneratedParameter::VerificationBranchTimeoutSecs)
    {
        decision.applied = required;
        decision.source = TuningSource::RaisedByVerificationInvariant;
    }
}

#[cfg(test)]
#[path = "generated_tuning_tests.rs"]
mod tests;
