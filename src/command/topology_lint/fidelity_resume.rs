//! The set gate's fidelity audit across attempts (Issue 259).
//!
//! Every critic call watches its own provider stream for no progress. Each
//! verdict that passes is saved as soon as its reply is read, one record per
//! obligation (#434, #434d), and each durable verdict reports progress to
//! the host.
//! A stalled call leaves every saved verdict intact and returns
//! `LintIncomplete` (exit 75); retries reuse those verdicts. Neither the
//! audit nor the host imposes a total deadline.

use anyhow::Result;
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, SkeletonSummary, fidelity_cluster_digest,
};
use archon_workflow::llm_client_port::WorkflowLlmClient;
use futures_util::{StreamExt, TryStreamExt};

use super::fidelity_critic::{Asked, ask};
#[cfg(test)]
use super::fidelity_store::StoreIdentity;
use super::fidelity_store::{Question, Standing, TaskTexts, VerdictStore, task_texts};
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeProgress, FreezeResume};

/// How every [`LintIncomplete`] text starts.
pub(crate) const LINT_INCOMPLETE_RESUMABLE: &str =
    "operational: task-set lint incomplete, resumable";
/// Batches in flight at once. Enough to overlap the provider's latency,
/// few enough that a serving endpoint sized for one decomposition is not
/// asked to hold sixteen long prompts at the same time.
const FIDELITY_CONCURRENCY: usize = 4;
/// Most obligations asked in one call. A cluster of one task claiming
/// twenty-three ids drew replies that dropped an id or invented one, twice;
/// eight verdicts per reply is a size the critic answers completely, and the
/// task texts it re-sends per batch are what provider prompt caches absorb.
/// Only the obligations of a cluster that have no standing verdict are
/// batched (#434).
const FIDELITY_MAX_OBLIGATIONS_PER_CALL: usize = 8;

/// One cluster: its obligations and the tasks every one of them is asked over.
pub(super) type Cluster = (Vec<ClaimedObligation>, Vec<ClaimingTask>);

/// What stands for every cluster's obligations, in cluster and obligation
/// order, and how many obligations were asked, in how many call batches,
/// and how many were served from the store.
pub(super) struct Resolved {
    pub(super) standings: Vec<Vec<Standing>>,
    pub(super) asked: usize,
    pub(super) batches: usize,
    pub(super) cached: usize,
}

/// One call batch: its cluster, the indexes of its obligations in that
/// cluster, the obligations, and its digest (which names rejected replies).
type Batch = (usize, Vec<usize>, Vec<ClaimedObligation>, String);

/// The audit stalled, with every answered verdict saved.
#[derive(Debug)]
pub(crate) struct LintIncomplete {
    outer_secs: u64,
    saved: u64,
    reused: u64,
    stopped: Vec<String>,
    progress: String,
}

impl LintIncomplete {
    fn new(budget: &FreezeBudget, progress: &FreezeProgress, stopped: Vec<String>) -> Self {
        Self {
            outer_secs: budget.outer_secs(),
            saved: progress.saved_count(),
            reused: progress.reused_count(),
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
        let host = if self.outer_secs == 0 {
            "no enclosing host call".to_string()
        } else {
            format!("{}s host no-progress window", self.outer_secs)
        };
        write!(
            f,
            "{LINT_INCOMPLETE_RESUMABLE}: a critic call made no provider progress ({host}); {} obligation verdict(s) were saved by this attempt and {} reused from earlier ones; {} batch(es) stalled without a verdict ({}). Retry the set gate with the same task set (resume the run): it continues from the saved verdicts",
            self.saved,
            self.reused,
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

/// Resolve every obligation from the store or a critic stream, saving each
/// answer as it arrives (#434: per obligation, not per batch). A silent
/// stream is resumable; no active batch is left unasked because of elapsed
/// work in other batches.
pub(super) async fn resolve(
    client: &dyn WorkflowLlmClient,
    store: &VerdictStore,
    clusters: &[Cluster],
    skeleton: &SkeletonSummary,
    resume: &FreezeResume,
) -> Result<Resolved> {
    // The envelope of the question with nothing in it: the prompt's own
    // wording, the skeleton section and every request setting, none of the
    // texts a verdict is anchored to.
    let store = store.for_request(
        client.message_request_identity(&super::fidelity_critic::request(&[], &[], skeleton)),
    );
    let store = &store;
    let legacy_records = store.legacy_records();
    let skeleton_digest = fidelity_cluster_digest(&[], &[], skeleton);
    let texts: Vec<TaskTexts> = clusters
        .iter()
        .map(|(_, tasks)| task_texts(tasks))
        .collect();
    let question = |cluster: usize, obligation: usize| Question {
        obligation: &clusters[cluster].0[obligation],
        skeleton: &skeleton_digest,
        tasks: &clusters[cluster].1,
        texts: &texts[cluster],
    };
    let mut resolved: Vec<Vec<Option<Standing>>> = clusters
        .iter()
        .enumerate()
        .map(|(cluster, (obligations, _))| {
            (0..obligations.len())
                .map(|obligation| {
                    store.load_or_migrate(question(cluster, obligation), &legacy_records)
                })
                .collect()
        })
        .collect();
    let cached = resolved
        .iter()
        .flatten()
        .filter(|entry| entry.is_some())
        .count();
    for _ in 0..cached {
        resume.progress.reused(false);
    }
    let batches: Vec<Batch> = resolved
        .iter()
        .enumerate()
        .flat_map(|(cluster, standings)| {
            let open: Vec<usize> = (0..standings.len())
                .filter(|index| standings[*index].is_none())
                .collect();
            open.chunks(FIDELITY_MAX_OBLIGATIONS_PER_CALL)
                .map(|indexes| {
                    let (obligations, tasks) = &clusters[cluster];
                    let asked: Vec<ClaimedObligation> =
                        indexes.iter().map(|i| obligations[*i].clone()).collect();
                    let digest = fidelity_cluster_digest(&asked, tasks, skeleton);
                    (cluster, indexes.to_vec(), asked, digest)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let asked = batches.iter().map(|batch| batch.1.len()).sum();
    let pending =
        batches
            .iter()
            .enumerate()
            .map(|(index, (cluster, indexes, obligations, digest))| {
                let question = &question;
                async move {
                    let tasks = &clusters[*cluster].1;
                    let budget = &resume.budget;
                    // Each verdict the critic gives that passes every check
                    // is saved as soon as its reply is read (#434d), so a
                    // batch that ends without the rest keeps it.
                    let mut standings: Vec<Option<Standing>> = vec![None; indexes.len()];
                    let mut accept = |position: usize, verdict: &FidelityVerdict| -> Result<()> {
                        let (standing, saved) =
                            store.save(question(*cluster, indexes[position]), verdict)?;
                        if saved {
                            resume.progress.saved(false);
                        }
                        // A verdict of the other meaning for an obligation
                        // already answered in this call is a contradiction,
                        // whether or not the store kept either one.
                        let standing = match (standings[position].take(), standing) {
                            (Some(Standing::Verdict(kept)), Standing::Verdict(new))
                                if kept.necessarily_true != new.necessarily_true =>
                            {
                                let (refuted, supported) = if new.necessarily_true {
                                    (kept, new)
                                } else {
                                    (new, kept)
                                };
                                Standing::Contradiction { refuted, supported }
                            }
                            (_, standing) => standing,
                        };
                        standings[position] = Some(standing);
                        Ok(())
                    };
                    let asked = ask(
                        client,
                        store.dir(),
                        digest,
                        obligations,
                        tasks,
                        skeleton,
                        budget,
                        &mut accept,
                    )
                    .await?;
                    let stopped = matches!(asked, Asked::Stopped);
                    Ok::<_, anyhow::Error>((index, stopped, standings))
                }
            });
    let answered: Vec<(usize, bool, Vec<Option<Standing>>)> = futures_util::stream::iter(pending)
        .buffer_unordered(FIDELITY_CONCURRENCY)
        .try_collect()
        .await?;
    let mut stopped = Vec::new();
    for (index, halted, standings) in answered {
        let (cluster, indexes, obligations, _) = &batches[index];
        let mut open = Vec::new();
        for ((obligation, standing), asked) in indexes.iter().zip(standings).zip(obligations) {
            match standing {
                Some(standing) => resolved[*cluster][*obligation] = Some(standing),
                None => open.push(asked.id.as_str()),
            }
        }
        if halted {
            // Named by the obligations it left without a verdict.
            stopped.push((index, open.join("+")));
        }
    }
    if !stopped.is_empty() {
        stopped.sort_unstable();
        return Err(LintIncomplete::new(
            &resume.budget,
            &resume.progress,
            stopped.into_iter().map(|(_, name)| name).collect(),
        )
        .into());
    }
    let standings = resolved
        .into_iter()
        .map(|cluster| {
            cluster
                .into_iter()
                .map(|standing| {
                    standing.expect("every obligation resolved from the store or the critic")
                })
                .collect()
        })
        .collect();
    Ok(Resolved {
        standings,
        asked,
        batches: batches.len(),
        cached,
    })
}

#[cfg(test)]
#[path = "fidelity_resume_tests.rs"]
mod tests;
