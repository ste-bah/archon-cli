//! Ending a workflow session that has stopped making progress (Issue-213 C2d).
//!
//! The repeat-answer detector only REMINDED: an agent getting the same answer
//! however it phrased the question was told so once and then left to spin for
//! as long as its budget lasted. This is the stop. It needs two facts, both
//! read by the host and neither parsed from any tool's output:
//!
//! - the detector still reports the agent stalled AFTER its reminder, and
//! - the working tree has not changed over [`STALL_ROUNDS`] tool rounds since.
//!
//! The second fact is what separates spinning from work: an edit that returns
//! the same "ok" every time is progress when the tree moves. The tree is read
//! as a digest of `git status` plus `git diff HEAD` of the session's working
//! directory, together with the write count the write tools recorded, so a
//! directory that is not a repository still shows its writes.
//!
//! Only inside a workflow run: an interactive agent is reminded, never cut.
//! The session ends with [`archon_tools::NO_PROGRESS_STOP_MARKER`], which the
//! write layer treats as a host interruption: the partial work is kept and the
//! work re-asked, never judged.

use std::path::Path;

/// Tool rounds, after the reminder, that the tree may stay unchanged while the
/// answers keep repeating. One full novelty window: long enough to act on the
/// reminder, short enough that a spinning agent is not left for hours.
pub(super) const STALL_ROUNDS: u32 = 8;

/// A fingerprint of the working tree and the session's recorded writes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeState {
    tree: Option<u64>,
    writes: u64,
}

#[derive(Debug, Default)]
pub(super) struct ProgressStop {
    baseline: Option<TreeState>,
    unchanged_rounds: u32,
}

impl ProgressStop {
    /// Observe the end of one tool round. `Some(reason)` ends the session.
    pub(super) async fn after_round(
        &mut self,
        ctx: &archon_tools::tool::ToolContext,
    ) -> Option<String> {
        if ctx.run_store.is_none() && ctx.workflow_read_guard.is_none() {
            return None;
        }
        let key = archon_tools::repeat_tool_guard::ChainKey::of(ctx);
        if !archon_tools::repeat_tool_guard::REPEAT_TOOL_CHAINS.novelty_stalled(&key) {
            *self = Self::default();
            return None;
        }
        let agent = ctx.subagent_id.as_deref().unwrap_or_default();
        let state = TreeState {
            tree: tree_digest(&ctx.working_dir).await,
            writes: archon_tools::session_progress::writes(agent),
        };
        self.observe(state)?;
        Some(archon_tools::no_progress_stop_message(
            self.unchanged_rounds,
        ))
    }

    /// `Some(())` once the tree stayed unchanged for [`STALL_ROUNDS`] rounds.
    fn observe(&mut self, state: TreeState) -> Option<()> {
        if self.baseline.as_ref() != Some(&state) {
            self.baseline = Some(state);
            self.unchanged_rounds = 0;
            return None;
        }
        self.unchanged_rounds += 1;
        (self.unchanged_rounds >= STALL_ROUNDS).then_some(())
    }
}

/// FNV-1a over `git status --porcelain` and `git diff HEAD` of `dir`; `None`
/// when `dir` is not a repository or git cannot answer in time, which the
/// write count then stands in for.
async fn tree_digest(dir: &Path) -> Option<u64> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for args in [
        &["status", "--porcelain=v1", "--untracked-files=all"][..],
        &["diff", "HEAD", "--no-ext-diff", "--binary"][..],
    ] {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            tokio::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        if !output.status.success() {
            return None;
        }
        for byte in output.stdout.iter().chain(b"\0") {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(tree: u64, writes: u64) -> TreeState {
        TreeState {
            tree: Some(tree),
            writes,
        }
    }

    #[test]
    fn an_unchanged_tree_is_stopped_after_the_stall_rounds() {
        let mut stop = ProgressStop::default();
        assert!(
            stop.observe(state(1, 0)).is_none(),
            "the first round sets the baseline"
        );
        for _ in 1..STALL_ROUNDS {
            assert!(stop.observe(state(1, 0)).is_none());
        }
        assert!(stop.observe(state(1, 0)).is_some());
    }

    #[test]
    fn a_changing_tree_or_a_new_write_restarts_the_count() {
        let mut stop = ProgressStop::default();
        stop.observe(state(1, 0));
        for _ in 1..STALL_ROUNDS {
            stop.observe(state(1, 0));
        }
        assert!(stop.observe(state(2, 0)).is_none(), "the tree moved");
        for _ in 1..STALL_ROUNDS {
            assert!(stop.observe(state(2, 0)).is_none());
        }
        assert!(stop.observe(state(2, 1)).is_none(), "a write was recorded");
    }

    #[tokio::test]
    async fn a_directory_outside_any_repository_has_no_tree_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(tree_digest(dir.path()).await, None);
    }

    #[tokio::test]
    async fn an_interactive_session_is_never_stopped() {
        let ctx = archon_tools::tool::ToolContext::default();
        let mut stop = ProgressStop::default();
        assert!(stop.after_round(&ctx).await.is_none());
    }
}
