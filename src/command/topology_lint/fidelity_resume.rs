//! The set gate's fidelity audit across attempts (Issue 259).
//!
//! The host kills `task-set-lint` at its catalog wall clock. The audit's
//! unit of work is one call batch: at most eight obligations of one cluster
//! asked of the critic in one provider call, up to four in flight. Here:
//!
//! - a batch already answered is read from the [`VerdictStore`] (keyed by
//!   its question, the binary and the critic) and never asked again;
//! - each batch the critic answers is saved the moment it is answered;
//! - before every critic call the staged gate's budget
//!   (`workflow_freeze_budget`, read from the same catalog entry less its
//!   safety margin) says how long the call may run; a call it would not
//!   allow is not started, and a call it cut short is not an error;
//! - when any batch is left without a verdict the audit ends with
//!   [`LintIncomplete`] rather than an evaluation, and the staged gate exits
//!   by the host's operational contract
//!   (`workflow_host_command_operational`): the reason, then the progress
//!   line, on stderr, and exit status 75. The executor retries the call while
//!   the progress grows.
//!
//! Progress counts batches with a saved verdict for this call: those this
//! attempt saved plus those it found saved. A progress line is written after
//! each one, so even an attempt the host kills reports how far it got.

use anyhow::Result;
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, SkeletonSummary,
};
use archon_workflow::llm_client_port::WorkflowLlmClient;
use futures_util::{StreamExt, TryStreamExt};

use super::fidelity_critic::{Asked, ask};
use super::fidelity_store::VerdictStore;
#[cfg(test)]
use super::fidelity_store::{StoreIdentity, store_dir};
use crate::command::workflow_freeze_budget::{
    FREEZE_SAFETY_MARGIN_SECS, FreezeBudget, FreezeProgress, FreezeResume,
};

/// How every [`LintIncomplete`] text starts.
pub(crate) const LINT_INCOMPLETE_RESUMABLE: &str =
    "operational: task-set lint incomplete, resumable";
/// Batches in flight at once. Enough to overlap the provider's latency,
/// few enough that a serving endpoint sized for one decomposition is not
/// asked to hold sixteen long prompts at the same time.
const FIDELITY_CONCURRENCY: usize = 4;

/// One call batch: its obligations, its cluster's tasks, its digest.
pub(super) type Batch = (Vec<ClaimedObligation>, Vec<ClaimingTask>, String);

/// Every batch's verdicts, in batch order.
pub(super) struct Resolved {
    pub(super) verdicts: Vec<Vec<FidelityVerdict>>,
    pub(super) asked: usize,
    pub(super) cached: usize,
}

/// The audit stopped for its time budget, with every answered batch saved.
#[derive(Debug)]
pub(crate) struct LintIncomplete {
    outer_secs: u64,
    saved: u64,
    reused: u64,
    not_started: Vec<String>,
    stopped: Vec<String>,
    progress: String,
}

impl LintIncomplete {
    fn new(
        budget: &FreezeBudget,
        progress: &FreezeProgress,
        not_started: Vec<String>,
        stopped: Vec<String>,
    ) -> Self {
        Self {
            outer_secs: budget.outer_secs(),
            saved: progress.saved_count(),
            reused: progress.reused_count(),
            not_started,
            stopped,
            progress: progress.line(),
        }
    }

    /// What the staged gate writes to stderr before it exits: the reason,
    /// then the progress line, last, as the host reads it.
    pub(crate) fn report(&self) -> String {
        format!("{self}\n{}", self.progress)
    }

    /// The incomplete audit `error` carries, if any.
    pub(crate) fn caused(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for LintIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{LINT_INCOMPLETE_RESUMABLE}: the set gate stops its critic calls {FREEZE_SAFETY_MARGIN_SECS}s before its {}s host wall clock; {} call batch verdict(s) were saved by this attempt and {} reused from earlier ones; {} batch(es) were not started for lack of time ({}) and {} were stopped at that deadline with their call still running ({}). Retry the set gate with the same task set (resume the run): it continues from the saved verdicts",
            self.outer_secs,
            self.saved,
            self.reused,
            self.not_started.len(),
            self.not_started.join(", "),
            self.stopped.len(),
            self.stopped.join(", ")
        )
    }
}

impl std::error::Error for LintIncomplete {}

/// The staged gate's ending for `error`, when it is an incomplete audit:
/// the stderr text and the exit status the host reads as incomplete,
/// resumable. `None` for every other error.
pub(crate) fn resumable_exit(error: &anyhow::Error) -> Option<(String, i32)> {
    LintIncomplete::caused(error).map(|incomplete| {
        (
            incomplete.report(),
            crate::command::workflow_host_command_operational::EXIT_INCOMPLETE_RESUMABLE,
        )
    })
}

/// Resolve every batch of `inputs`: from `store`, else from the critic
/// under `resume`'s budget, saving each answer as it arrives. A critic
/// failure is an error, as before; a batch the budget left unasked makes
/// the result [`LintIncomplete`].
pub(super) async fn resolve(
    client: &dyn WorkflowLlmClient,
    store: &VerdictStore,
    inputs: &[Batch],
    skeleton: &SkeletonSummary,
    resume: &FreezeResume,
) -> Result<Resolved> {
    let stores: Vec<_> =
        inputs
            .iter()
            .map(|(obligations, tasks, _)| {
                store.for_request(client.message_request_identity(
                    &super::fidelity_critic::request(obligations, tasks, skeleton),
                ))
            })
            .collect();
    let mut resolved: Vec<Option<Vec<FidelityVerdict>>> = inputs
        .iter()
        .zip(&stores)
        .map(|((obligations, tasks, digest), store)| store.load(digest, obligations, tasks))
        .collect();
    let cached = resolved.iter().filter(|entry| entry.is_some()).count();
    for _ in 0..cached {
        resume.progress.reused(false);
    }
    let asked = resolved.len() - cached;
    let pending = inputs
        .iter()
        .enumerate()
        .filter(|(index, _)| resolved[*index].is_none())
        .map(|(index, (obligations, tasks, digest))| {
            let store = &stores[index];
            async move {
                let budget = &resume.budget;
                let dir = store.dir();
                let asked = ask(client, dir, digest, obligations, tasks, skeleton, budget).await?;
                if let Asked::Answered(verdicts) = &asked
                    && store.save(digest, verdicts)?
                {
                    resume.progress.saved(false);
                }
                Ok::<_, anyhow::Error>((index, asked))
            }
        });
    let answered: Vec<(usize, Asked)> = futures_util::stream::iter(pending)
        .buffer_unordered(FIDELITY_CONCURRENCY)
        .try_collect()
        .await?;
    let name = |index: usize| {
        let ids: Vec<&str> = inputs[index].0.iter().map(|o| o.id.as_str()).collect();
        ids.join("+")
    };
    let (mut not_started, mut stopped) = (Vec::new(), Vec::new());
    for (index, asked) in answered {
        match asked {
            Asked::Answered(verdicts) => resolved[index] = Some(verdicts),
            Asked::NotStarted => not_started.push(index),
            Asked::Stopped => stopped.push(index),
        }
    }
    if !(not_started.is_empty() && stopped.is_empty()) {
        not_started.sort_unstable();
        stopped.sort_unstable();
        return Err(LintIncomplete::new(
            &resume.budget,
            &resume.progress,
            not_started.into_iter().map(name).collect(),
            stopped.into_iter().map(name).collect(),
        )
        .into());
    }
    let verdicts = resolved
        .into_iter()
        .map(|verdicts| verdicts.expect("every batch resolved from the store or the critic"))
        .collect();
    Ok(Resolved {
        verdicts,
        asked,
        cached,
    })
}

#[cfg(test)]
#[path = "fidelity_resume_tests.rs"]
mod tests;
