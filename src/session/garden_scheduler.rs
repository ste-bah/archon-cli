//! The scheduling seam: when an unattended consolidation pass may run.
//!
//! Everything about *what* such a pass is allowed to do lives in
//! `archon_memory::garden` — the run lock, the work budget, and the rule that a
//! scheduled pass proposes retirements rather than performing them. This file
//! owns only the timing, and the one thing the memory crate cannot do for
//! itself: writing the proposals somewhere a person can find them.
//!
//! # Off by default
//!
//! [`spawn_garden_scheduler`] returns `None` unless
//! `memory.garden.scheduled_consolidation` is switched on, and that setting
//! defaults to false. Unattended maintenance of a user's stored memories is not
//! something to opt people into on upgrade.
//!
//! # Why the spawn function is synchronous
//!
//! The same reason `spawn_review_band_adjudication` is. This is called from
//! session bootstrap, before the TUI is up and before the user can type; every
//! `.await` there is time spent looking at a splash screen. Being a plain `fn`
//! is not an accident of style — an `async fn` here could be awaited back into
//! the bootstrap by a later edit and nobody would notice, because the symptom is
//! a slower launch rather than a failure.
//!
//! # Why the timer ticks faster than the interval
//!
//! The tick is a *check*, not the schedule. The schedule lives in the store, as
//! the `garden:last_run` timestamp that `should_auto_consolidate` reads, because
//! that is the only place several Archon processes sharing one store can agree
//! on it. A process-local "every 24 hours" timer would restart on every launch,
//! so on a machine where Archon is opened and closed daily it would never fire
//! at all — and on a machine running several at once, each would keep its own
//! idea of when the last pass was.
//!
//! # What a failure leaves behind
//!
//! Nothing to clean up. The run lock is an OS advisory lock released by the
//! kernel when the process dies. The pass writes whole units and takes its
//! budget decision before each one. Proposals that fail to persist are simply
//! not persisted — the memories they name are untouched either way, and the next
//! pass re-derives the same candidates from the same store.

use std::path::PathBuf;
use std::sync::Arc;

use archon_learning::garden_proposals::{
    GardenProposalKind, GardenProposalRecord, GardenProposalStatus, raise_garden_proposal,
};
use archon_memory::MemoryTrait;
use archon_memory::garden::{
    GardenConfig, RetirementCandidate, RetirementReason, RuleRetirementCandidate,
    RuleRetirementPolicy, ScheduledRun, SemanticConsolidationCandidate, rule_retirement_candidates,
    run_scheduled_consolidation, should_run_scheduled,
};

use crate::command::garden_metrics::{GardenMetricContext, record_proposal_raised};

/// How often the scheduler asks whether a pass is due.
///
/// Not the interval between passes — see the module docs. Fifteen minutes is
/// short enough that a session open across the moment a pass becomes due picks
/// it up promptly, and long enough that the check itself (one indexed read of
/// one row) is not worth measuring.
const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Everything the background scheduler needs, gathered at the call site.
///
/// A struct rather than eight arguments, so adding a dependency later does not
/// silently reorder two `Arc`s of similar type at the one call site.
pub(crate) struct GardenSchedulerSpec {
    pub garden: GardenConfig,
    pub memory: Arc<dyn MemoryTrait>,
    /// Where `memory.port` and the run lock live. This is what makes two Archon
    /// processes agree they are looking at one store.
    pub data_dir: PathBuf,
    /// The project root, for the cognitive metric ledger and the policy that
    /// segments its cohorts.
    pub working_dir: PathBuf,
    /// The governed-learning store retirement proposals are written to.
    ///
    /// `None` disables proposal persistence, not the pass. A pass with nowhere
    /// to file its proposals still decays and merges — both reversible — and
    /// still refuses to delete. Losing the proposals costs a review pile that
    /// the next pass rebuilds; it costs no memory.
    pub learning_db: Option<Arc<cozo::DbInstance>>,
}

/// Start the unattended consolidation scheduler, if it is switched on.
///
/// SYNCHRONOUS, and returns before any store has been touched. Returns the
/// detached task, or `None` when scheduled consolidation is off — which is the
/// default. Production drops the handle, which detaches it; it is returned so a
/// test can join a tick instead of racing it.
pub(crate) fn spawn_garden_scheduler(
    spec: GardenSchedulerSpec,
) -> Option<tokio::task::JoinHandle<()>> {
    if !should_run_scheduled(&spec.garden) {
        return None;
    }
    tracing::info!(
        interval_hours = spec.garden.scheduled_interval_hours,
        check_interval_secs = CHECK_INTERVAL.as_secs(),
        proposals_persisted = spec.learning_db.is_some(),
        "garden: scheduled consolidation enabled"
    );
    Some(tokio::spawn(async move {
        let GardenSchedulerSpec {
            garden,
            memory,
            data_dir,
            working_dir,
            learning_db,
        } = spec;
        let mut ticker = tokio::time::interval(CHECK_INTERVAL);
        // The first tick of a tokio interval fires immediately. Consumed here so
        // the scheduler does not race the session-start consolidation that is
        // still finishing a few lines above its own call site: they would
        // contend on the run lock, one would be declined, and the declined one
        // would be whichever lost -- a coin flip is a poor way to decide whether
        // the user's startup pass or a background one runs.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            run_one_tick(
                &garden,
                &memory,
                &data_dir,
                &working_dir,
                learning_db.as_deref(),
            )
            .await;
        }
    }))
}

/// One check, and the pass it may produce.
///
/// Separated from the loop so it can be driven directly by a test without a
/// timer, and so the loop above holds nothing but timing.
async fn run_one_tick(
    garden: &GardenConfig,
    memory: &Arc<dyn MemoryTrait>,
    data_dir: &std::path::Path,
    working_dir: &std::path::Path,
    learning_db: Option<&cozo::DbInstance>,
) {
    let run_id = format!("garden-scheduled:{}", uuid::Uuid::new_v4());
    let memory_for_task = Arc::clone(memory);
    let garden_for_task = garden.clone();
    let data_dir_for_task = data_dir.to_path_buf();
    let id_for_task = run_id.clone();

    // `run_scheduled_consolidation` is synchronous and does database work, over
    // TCP in every process but the one that owns the store. Running it on the
    // async runtime's worker threads would block them for the length of a pass.
    let outcome = tokio::task::spawn_blocking(move || {
        run_scheduled_consolidation(
            memory_for_task.as_ref(),
            &garden_for_task,
            &data_dir_for_task,
            &id_for_task,
        )
    })
    .await;

    let outcome = match outcome {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            tracing::warn!(%error, run_id, "garden: scheduled consolidation failed");
            return;
        }
        Err(error) => {
            // The blocking task panicked. Logged rather than propagated: the
            // store is consistent regardless -- the pass writes whole units --
            // and taking the session down because a maintenance pass failed
            // would be a worse outcome than skipping the pass.
            tracing::warn!(%error, run_id, "garden: scheduled consolidation task did not complete");
            return;
        }
    };

    let report = match outcome {
        ScheduledRun::Ran(report) => report,
        ScheduledRun::Declined | ScheduledRun::TooRecent => return,
    };

    // Rule retirement is computed here rather than inside the pass, because the
    // evidence lives in two places the memory crate deliberately cannot join:
    // the rules engine and the correction rows behind each rule. The analysis
    // itself is a pure function taking plain data, so nothing on this path can
    // reach a rule-mutating call.
    let rule_candidates = rule_retirement_candidates(
        &super::garden_rule_observations::rule_observations(memory.as_ref()),
        &RuleRetirementPolicy::default(),
        chrono::Utc::now(),
    );

    let total = report.retirement_candidates.len()
        + report.consolidation_candidates.len()
        + rule_candidates.len();
    if total == 0 {
        return;
    }
    let Some(db) = learning_db else {
        tracing::info!(
            run_id,
            candidates = total,
            "garden: proposals found but no governed store is open; nothing was \
             changed and nothing was recorded"
        );
        return;
    };
    let metrics = GardenMetricContext {
        working_dir: working_dir.to_path_buf(),
        model_id: "scheduler".to_string(),
        session_id: run_id.clone(),
        turn_number: 0,
    };
    let filed = file_retirements(db, &run_id, &report.retirement_candidates, &metrics)
        + file_consolidations(db, &run_id, &report.consolidation_candidates, &metrics)
        + file_rule_retirements(db, &run_id, &rule_candidates, &metrics);
    tracing::info!(
        run_id,
        filed,
        offered = total,
        "garden: proposals filed for review; nothing was applied"
    );
}

/// Raise one proposal and record that it was raised.
///
/// Best effort per proposal. One that fails to persist is logged and skipped
/// rather than aborting the batch: the store is untouched either way, so a
/// partial review pile is strictly better than none and the next pass
/// re-derives whatever was missed.
fn raise(
    db: &cozo::DbInstance,
    record: GardenProposalRecord,
    metrics: &GardenMetricContext,
) -> usize {
    match raise_garden_proposal(db, &record) {
        Ok(stored) => {
            // Only a genuinely new pending row is counted and measured. A
            // proposal that was already decided comes back unchanged, and
            // counting it would inflate the acceptance denominator with rows
            // nobody was asked about again.
            if stored.status == GardenProposalStatus::Pending && stored.run_id == record.run_id {
                record_proposal_raised(metrics, &stored);
                return 1;
            }
            0
        }
        Err(error) => {
            tracing::warn!(
                %error,
                subject = %record.subject_id,
                "garden: could not file a proposal; the store is untouched"
            );
            0
        }
    }
}

fn file_retirements(
    db: &cozo::DbInstance,
    run_id: &str,
    candidates: &[RetirementCandidate],
    metrics: &GardenMetricContext,
) -> usize {
    let created_at = chrono::Utc::now().to_rfc3339();
    candidates
        .iter()
        .map(|candidate| {
            raise(
                db,
                GardenProposalRecord {
                    proposal_id: GardenProposalRecord::stable_id(
                        GardenProposalKind::MemoryRetirement,
                        &candidate.memory_id,
                    ),
                    proposal_kind: GardenProposalKind::MemoryRetirement,
                    subject_id: candidate.memory_id.clone(),
                    subject_title: candidate.title.clone(),
                    excerpt: candidate.excerpt.clone(),
                    detail: describe_reason(&candidate.reason),
                    payload_json: "{}".to_string(),
                    run_id: run_id.to_string(),
                    status: GardenProposalStatus::Pending,
                    applied_ref: String::new(),
                    created_at: created_at.clone(),
                    decided_at: String::new(),
                },
                metrics,
            )
        })
        .sum()
}

fn file_consolidations(
    db: &cozo::DbInstance,
    run_id: &str,
    candidates: &[SemanticConsolidationCandidate],
    metrics: &GardenMetricContext,
) -> usize {
    let created_at = chrono::Utc::now().to_rfc3339();
    candidates
        .iter()
        .map(|candidate| {
            // The whole candidate travels as the payload, because applying it
            // has to write the exact text and sources that were reviewed. A
            // proposal that re-derived its content at apply time could apply
            // something other than what was approved.
            let payload = serde_json::to_string(candidate).unwrap_or_else(|error| {
                tracing::warn!(%error, "garden: consolidation payload not serialisable");
                String::new()
            });
            if payload.is_empty() {
                return 0;
            }
            raise(
                db,
                GardenProposalRecord {
                    proposal_id: GardenProposalRecord::stable_id(
                        GardenProposalKind::SemanticConsolidation,
                        &candidate.candidate_id,
                    ),
                    proposal_kind: GardenProposalKind::SemanticConsolidation,
                    subject_id: candidate.candidate_id.clone(),
                    subject_title: candidate.proposed_title.clone(),
                    excerpt: candidate.proposed_content.clone(),
                    detail: format!(
                        "{} provenance-compatible memories restate this claim; \
                         recording it once at importance {:.2}",
                        candidate.corroboration_count(),
                        candidate.proposed_importance
                    ),
                    payload_json: payload,
                    run_id: run_id.to_string(),
                    status: GardenProposalStatus::Pending,
                    applied_ref: String::new(),
                    created_at: created_at.clone(),
                    decided_at: String::new(),
                },
                metrics,
            )
        })
        .sum()
}

fn file_rule_retirements(
    db: &cozo::DbInstance,
    run_id: &str,
    candidates: &[RuleRetirementCandidate],
    metrics: &GardenMetricContext,
) -> usize {
    let created_at = chrono::Utc::now().to_rfc3339();
    candidates
        .iter()
        .map(|candidate| {
            raise(
                db,
                GardenProposalRecord {
                    proposal_id: GardenProposalRecord::stable_id(
                        GardenProposalKind::RuleRetirement,
                        &candidate.rule_id,
                    ),
                    proposal_kind: GardenProposalKind::RuleRetirement,
                    subject_id: candidate.rule_id.clone(),
                    subject_title: "behavioural rule".to_string(),
                    excerpt: candidate.rule_text.clone(),
                    detail: describe_rule_evidence(candidate),
                    payload_json: "{}".to_string(),
                    run_id: run_id.to_string(),
                    status: GardenProposalStatus::Pending,
                    applied_ref: String::new(),
                    created_at: created_at.clone(),
                    decided_at: String::new(),
                },
                metrics,
            )
        })
        .sum()
}

/// The correction evidence behind a rule-retirement proposal, as one line.
fn describe_rule_evidence(candidate: &RuleRetirementCandidate) -> String {
    let evidence = &candidate.evidence;
    let corrections = match evidence.days_since_supporting_correction {
        Some(days) => format!("last supporting correction {days} days ago"),
        None => "no supporting correction on record".to_string(),
    };
    let triggered = match evidence.days_since_triggered {
        Some(days) => format!("last matched {days} days ago"),
        None => "never matched".to_string(),
    };
    format!(
        "{corrections}, {triggered}, {} supporting correction(s), score {:.1}; \
         quiet threshold {} days",
        evidence.supporting_corrections, evidence.score, evidence.quiet_days
    )
}

/// The evidence behind a candidate, as one readable line.
///
/// Rendered here rather than stored as structured columns because the reader is
/// a person deciding about one memory, and the numbers differ per reason. The
/// machine-readable half a caller might group by is `reason_kind`, which is a
/// column.
fn describe_reason(reason: &RetirementReason) -> String {
    match reason {
        RetirementReason::Stale {
            days_since_access,
            staleness_days,
            importance_floor,
        } => format!(
            "untouched for {days_since_access} days (threshold {staleness_days}) \
             and below the importance floor of {importance_floor:.2}"
        ),
        RetirementReason::Overflow {
            max_memories,
            total_memories,
        } => format!(
            "store holds {total_memories} memories against a cap of {max_memories}; \
             this row is among the least important"
        ),
    }
}

#[cfg(test)]
#[path = "garden_scheduler_tests.rs"]
mod tests;
