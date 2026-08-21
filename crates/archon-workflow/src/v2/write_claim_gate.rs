//! Decide a write conflict when the agent first touches a file, not when it
//! submits an hour of work.
//!
//! # The cost this removes
//!
//! Today a conflict is discovered at patch-apply time. On wf-3d7efd28,
//! `implementation-wave-1-impl-tdl-040` generated a complete patch against
//! `data_lake/identity.rs`, submitted it, and was told:
//!
//! ```text
//! stale baseline at crates/archon-trading/src/data_lake/identity.rs: the file
//! changed after this patch was computed and was NOT modified; re-read ... and
//! regenerate the change against current contents
//! ```
//!
//! The patch was correct when it was computed. The whole turn was discarded
//! because the answer arrived at the end. `graft` makes the general case of the
//! argument: worktrees "detect write conflicts at merge time, after all agents
//! have finished", while a claim checked at write time surfaces the same
//! conflict "mid-execution", when the agent can still wait, pivot, or re-read
//! the file for the cost of one tool call.
//!
//! # What this is not
//!
//! It is not a lock and it does not serialise anything. `write_mode::plan`
//! already partitions a stage into waves with disjoint ownership, and waves run
//! sequentially — that machinery works and this does not replace it. This gate
//! answers a narrower question the planner cannot: the file an agent is about to
//! write moved AFTER its worktree baseline was taken, so the patch it is
//! building will not apply. Better to say so at the first write than at the
//! last.
//!
//! The decision is deliberately advisory-shaped. `ContextNest`'s lease plane
//! defaults to advisory because "a fleet of cooperating agents" is the common
//! case and a hard denial mid-turn is expensive; the strict outcome is reserved
//! for a claim someone else genuinely holds.

/// What an agent should do about the file it is about to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteClaimDecision {
    /// The baseline is current. Proceed.
    Proceed,
    /// The file moved since this branch's baseline was taken. Any patch built
    /// on the stale contents will be rejected at apply time, so re-read now
    /// while it costs one tool call instead of a whole turn.
    Restale {
        path: String,
        baseline_digest: String,
        current_digest: String,
    },
}

impl WriteClaimDecision {
    pub fn should_proceed(&self) -> bool {
        matches!(self, Self::Proceed)
    }

    /// The message handed to the agent. Phrased as an instruction rather than a
    /// rejection: the work is not lost yet, and the agent can still act.
    pub fn guidance(&self) -> Option<String> {
        match self {
            Self::Proceed => None,
            Self::Restale { path, .. } => Some(format!(
                "'{path}' changed after your branch baseline was taken. Re-read \
                 '{path}' as it is NOW before editing it — a patch computed \
                 against the contents you already have will be rejected at apply \
                 time and the work discarded."
            )),
        }
    }
}

/// Compare a file's digest at branch baseline against its digest right now.
///
/// `baseline` is what the branch recorded when its worktree was created;
/// `current` is the digest of the canonical file at this moment. `None` for
/// either means the file did not exist at that point: created-since and
/// deleted-since are both real divergence, and a file absent at both ends is
/// simply new and free to write.
pub fn decide_write_claim(
    path: &str,
    baseline: Option<&str>,
    current: Option<&str>,
) -> WriteClaimDecision {
    match (baseline, current) {
        (None, None) => WriteClaimDecision::Proceed,
        (Some(baseline), Some(current)) if baseline == current => WriteClaimDecision::Proceed,
        (baseline, current) => WriteClaimDecision::Restale {
            path: path.to_string(),
            baseline_digest: baseline.unwrap_or("<absent>").to_string(),
            current_digest: current.unwrap_or("<absent>").to_string(),
        },
    }
}

#[cfg(test)]
#[path = "write_claim_gate_tests.rs"]
mod tests;
