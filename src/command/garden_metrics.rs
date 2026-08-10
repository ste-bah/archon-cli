//! Cognitive metric emission for governed memory-garden proposals.
//!
//! The R8 metric infrastructure already declares what a governed proposal is
//! worth measuring — acceptance, reversal, rule churn — and had no emitter for
//! any of it. Definitions with no writer derive nothing, and a gate over a
//! metric nobody emits returns `NotEvaluated`, which is not a pass.
//!
//! So every lifecycle step of a garden proposal writes one event here, on the
//! path that actually performs the step rather than in a reporting pass that
//! would have to reconstruct it.
//!
//! # Why apply and rollback carry a ratio rather than a flag
//!
//! Reversal rate is rollbacks over applications, and a metric event is
//! immutable: at the moment a change is applied, nothing knows whether it will
//! later be undone. Recording `reversed: false` at apply time would make the
//! rate a constant zero.
//!
//! So an application contributes `denominator = 1` and a rollback contributes
//! `numerator = 1`, both tagged `proposal_application_outcome = recorded`, and
//! the pooled ratio over that population is the reversal rate. Decisions carry
//! neither and are excluded by the identity filter rather than by relying on
//! them summing to nothing.
//!
//! # Failure never blocks the operation
//!
//! Every function here returns `()`. A metric that cannot be written is a
//! measurement lost; a retirement that fails because its measurement failed
//! would be a governed decision lost. The first is recoverable and the second is
//! not, so emission is best effort and says so in the log.

use std::path::Path;

use archon_cognitive::PersistentCognitiveStore;
use archon_cognitive::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use archon_cognitive::metrics::{MetricEmitter, runtime_cohort};
use archon_learning::garden_proposals::{GardenProposalKind, GardenProposalRecord};

/// Task class every garden proposal metric is segmented under.
///
/// One class rather than one per proposal kind: the kind is already an identity
/// on the event, and splitting the cohort by it would divide an already-small
/// population into three that each fall under the gate's minimum sample count.
const TASK_CLASS: &str = "memory_consolidation";

/// Where the metric ledger and cognitive relations live, relative to a project.
fn cognitive_root(working_dir: &Path) -> std::path::PathBuf {
    working_dir.join(".archon").join("cognitive")
}

/// Everything the emitters need that is not on the proposal itself.
///
/// Owned rather than borrowed. A borrowing form would hold the command context
/// immutably for as long as a batch of applications runs, and that context is
/// also what the loop emits progress through — so the lifetime would be paid
/// for in call-site contortions rather than in copies of three short strings.
#[derive(Debug, Clone)]
pub(crate) struct GardenMetricContext {
    pub working_dir: std::path::PathBuf,
    pub model_id: String,
    pub session_id: String,
    pub turn_number: u64,
}

/// A proposal was raised by a consolidation pass.
pub(crate) fn record_proposal_raised(
    context: &GardenMetricContext,
    proposal: &GardenProposalRecord,
) {
    emit(context, proposal, "raise", |event| event);
}

/// A person accepted or refused a proposal.
pub(crate) fn record_proposal_decided(
    context: &GardenMetricContext,
    proposal: &GardenProposalRecord,
    accepted: bool,
) {
    let decision = if accepted { "accepted" } else { "rejected" };
    emit(context, proposal, "decide", move |event| {
        event.with_identity("proposal_decision", decision)
    });
}

/// An approved proposal was applied to the store.
///
/// Also emits the rule-lifecycle event when the applied change retired a prompt
/// rule, which is what makes rule churn measurable: `rule_retire_count` had a
/// definition and no writer.
pub(crate) fn record_proposal_applied(
    context: &GardenMetricContext,
    proposal: &GardenProposalRecord,
    applied_ref: &str,
) {
    let reference = applied_ref.to_string();
    emit(context, proposal, "apply", move |event| {
        event
            .with_identity("proposal_application_outcome", "recorded")
            .with_identity("applied_ref", reference)
            // Denominator only: this application may or may not be reversed,
            // and the event cannot be rewritten when that is known.
            .with_ratio(0.0, 1.0)
    });
    if proposal.proposal_kind == GardenProposalKind::RuleRetirement {
        emit_rule_lifecycle(context, &proposal.subject_id, "retire");
    }
}

/// An applied proposal was undone.
pub(crate) fn record_proposal_rolled_back(
    context: &GardenMetricContext,
    proposal: &GardenProposalRecord,
) {
    emit(context, proposal, "rollback", |event| {
        event
            .with_identity("proposal_application_outcome", "recorded")
            // Numerator only: one reversal against the applications above.
            .with_ratio(1.0, 0.0)
    });
    if proposal.proposal_kind == GardenProposalKind::RuleRetirement {
        // A rule returning to the prompt is a lifecycle event in its own right.
        // Without it, churn would look one-directional and a rule retired and
        // restored ten times would read as ten retirements.
        emit_rule_lifecycle(context, &proposal.subject_id, "restore");
    }
}

fn emit(
    context: &GardenMetricContext,
    proposal: &GardenProposalRecord,
    operation: &str,
    decorate: impl FnOnce(CognitiveMetricEvent) -> CognitiveMetricEvent,
) {
    let subject = format!("{}:{operation}", proposal.proposal_id);
    let build = |emitter: &MetricEmitter<'_>| {
        decorate(
            emitter
                .event(
                    "governed_proposal_acceptance_rate",
                    MetricEventKind::GovernedProposalObserved,
                    &subject,
                    chrono::Utc::now(),
                )
                .with_session(context.session_id.as_str(), context.turn_number)
                .with_identity("governed_proposal_id", proposal.proposal_id.as_str())
                .with_identity("proposal_kind", proposal.proposal_kind.as_str())
                .with_identity("proposal_lifecycle_operation", operation)
                .with_identity("proposal_subject_id", proposal.subject_id.as_str()),
        )
    };
    write(context, build);
}

fn emit_rule_lifecycle(context: &GardenMetricContext, rule_id: &str, operation: &str) {
    let subject = format!("{rule_id}:{operation}");
    let rule_id = rule_id.to_string();
    let operation = operation.to_string();
    write(context, move |emitter: &MetricEmitter<'_>| {
        emitter
            .event(
                "rule_retire_count",
                MetricEventKind::RuleLifecycleObserved,
                &subject,
                chrono::Utc::now(),
            )
            .with_session(context.session_id.as_str(), context.turn_number)
            .with_identity("rule_id", rule_id)
            .with_identity("rule_operation", operation)
    });
}

/// Open the store once and write a batch of events.
///
/// Batched because one injection observation produces one row per consolidated
/// memory, and opening the cognitive store per row would put a file-system open
/// and a schema check on the prompt-building path for each of them.
///
/// `build` is called once with the emitter and returns every event to write, so
/// callers that need the emitter's window id can use it while building.
pub(crate) fn write_batch(
    context: &GardenMetricContext,
    build: impl FnOnce(&MetricEmitter<'_>) -> Vec<CognitiveMetricEvent>,
) {
    let root = cognitive_root(context.working_dir.as_path());
    let store = match PersistentCognitiveStore::open(&root) {
        Ok(store) => store,
        Err(error) => {
            tracing::debug!(%error, root = %root.display(), "garden: no cognitive store; metrics not recorded");
            return;
        }
    };
    let policy = archon_policy::load_effective_policy(context.working_dir.as_path())
        .ok()
        .map(|effective| effective.cognitive);
    let emitter = match MetricEmitter::open(
        store.db(),
        &root,
        runtime_cohort(TASK_CLASS, context.model_id.as_str(), policy.as_ref()),
    ) {
        Ok(emitter) => emitter,
        Err(error) => {
            tracing::warn!(%error, "garden: could not open the metric emitter");
            return;
        }
    };
    for event in build(&emitter) {
        if let Err(error) = emitter.record(&event) {
            // A same-id write with different content is how the store reports a
            // replay whose timestamp moved. It is benign -- the first write is
            // the observation -- so it is logged where it can be found rather
            // than warned about on a path that runs every turn.
            tracing::debug!(%error, metric = %event.metric_name, "garden: metric not recorded");
        }
    }
}

/// Open the store, build the event, write it. Every failure is a warning.
fn write(
    context: &GardenMetricContext,
    build: impl FnOnce(&MetricEmitter<'_>) -> CognitiveMetricEvent,
) {
    let root = cognitive_root(context.working_dir.as_path());
    let store = match PersistentCognitiveStore::open(&root) {
        Ok(store) => store,
        Err(error) => {
            tracing::debug!(%error, root = %root.display(), "garden: no cognitive store; metric not recorded");
            return;
        }
    };
    // Fails open to "no policy": an unreadable policy is a reason to segment
    // conservatively, not to drop the observation.
    let policy = archon_policy::load_effective_policy(context.working_dir.as_path())
        .ok()
        .map(|effective| effective.cognitive);
    let emitter = match MetricEmitter::open(
        store.db(),
        &root,
        runtime_cohort(TASK_CLASS, context.model_id.as_str(), policy.as_ref()),
    ) {
        Ok(emitter) => emitter,
        Err(error) => {
            tracing::warn!(%error, "garden: could not open the metric emitter");
            return;
        }
    };
    let event = build(&emitter);
    if let Err(error) = emitter.record(&event) {
        tracing::warn!(%error, metric = %event.metric_name, "garden: metric not recorded");
    }
}

#[cfg(test)]
#[path = "garden_metrics_tests.rs"]
mod tests;
