//! Coordination outcomes as world-model trace rows (#184 M9).
//!
//! Subagent activity already reached the learning systems through transcripts,
//! activity events and the usage ledger. What had no representation was the
//! coordination itself: an agent claiming work another agent was already
//! writing, and whether that turned into a conflicting merge an hour later.
//!
//! Merge results are the valuable half, because they are **ground truth**. A
//! git merge either conflicted or it did not — no labeler judged it, no heuristic
//! inferred it. That makes a row here trainable against directly, and it gives
//! the advisor a question worth asking at spawn time: overlapping claims on a
//! shared module mean serialize rather than parallelize.
//!
//! Best-effort by construction. A store that will not open, or a session with
//! no world-model root configured, must not fail the merge the operator asked
//! for — the merge already happened either way.

use std::path::Path;

use archon_tools::coordination_record::SpawnFacts;
use archon_world_model::schema::{WorldActionKind, WorldTraceRow, WorldTraceSource};
use archon_world_model::storage::WorldModelStore;

/// Where coordination rows are written.
///
/// The same user-global root every other world-model writer uses. `None` rather
/// than an error when the home directory is unavailable: a merge must not fail
/// because a learning signal has nowhere to go.
pub(crate) fn coordination_root() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(".archon").join("world-model"))
}

/// What happened when an isolated agent's branch was dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeOutcome {
    /// Integrated cleanly.
    Merged,
    /// Refused because the merge conflicted. The label this whole path exists for.
    Conflicted,
    /// Thrown away without merging.
    Discarded,
}

impl MergeOutcome {
    /// Read the outcome off what `exit_worktree` reported.
    ///
    /// Conflicts are distinguished from other failures because only a conflict
    /// is evidence about whether the work overlapped — a merge that failed
    /// because the repository could not be opened says nothing about the agents.
    pub(crate) fn classify(action: &str, result: &Result<String, String>) -> Option<Self> {
        match result {
            Ok(_) if action == "merge" => Some(Self::Merged),
            Ok(_) if action == "discard" => Some(Self::Discarded),
            Ok(_) => None,
            Err(error) if error.to_lowercase().contains("conflict") => Some(Self::Conflicted),
            Err(_) => None,
        }
    }
}

/// Record one isolated agent's merge outcome.
///
/// `files_changed` comes from the diffstat M7 already computes, which is what
/// the agent actually touched rather than what it said it would.
pub(crate) fn record_merge_outcome(
    root: &Path,
    session_id: &str,
    owner_id: &str,
    outcome: MergeOutcome,
    files_changed: usize,
) {
    // Consumed: the merge is the event that closes this agent's loop, so the
    // spawn facts have no further reader.
    let facts = archon_tools::coordination_record::take(owner_id).unwrap_or_default();
    if let Err(error) = persist(root, session_id, owner_id, outcome, files_changed, &facts) {
        tracing::warn!(owner_id, %error, "could not record the merge outcome for learning");
    }
}

fn persist(
    root: &Path,
    session_id: &str,
    owner_id: &str,
    outcome: MergeOutcome,
    files_changed: usize,
    facts: &SpawnFacts,
) -> anyhow::Result<()> {
    let store = WorldModelStore::open(root)?;

    let mut row = WorldTraceRow::new(session_id, WorldActionKind::WorktreeMerge)
        .with_row_id(format!("world-row-merge-{owner_id}"));
    row.source = WorldTraceSource::AgentOutput;
    row.agent = facts.label.clone();
    row.coordination_run_id = facts.coordination_run_id.clone();

    row.labels.merge_conflict = outcome == MergeOutcome::Conflicted;
    row.labels.claim_overlap = facts.claim_overlap;
    row.labels.isolated = facts.isolated;
    // A discard is the agent's work being thrown away, which is a failure of
    // the spawn whatever the reason. A conflict is not: the work exists, it
    // just did not apply.
    row.labels.failure = outcome == MergeOutcome::Discarded;
    row.labels.success = Some(outcome == MergeOutcome::Merged);

    // What it actually touched, not what it declared. `attempt_index` is the
    // nearest existing scalar slot for a count, and adding a field to
    // `ScalarFeatures` would shift nothing else usefully.
    row.scalar_features.attempt_index = Some(files_changed as u32);

    row.redacted_excerpt = Some(format!(
        "{} declared {} path(s); overlap={}; isolated={}; {} file(s) changed",
        facts.label.as_deref().unwrap_or(owner_id),
        facts.declared.len(),
        facts.claim_overlap,
        facts.isolated,
        files_changed,
    ));

    store.persist_rows(&[row])?;
    Ok(())
}

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod tests;
