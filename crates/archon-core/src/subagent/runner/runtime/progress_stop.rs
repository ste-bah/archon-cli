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
//! A second verdict needs no repeated answers at all: OSCILLATION. A tree that
//! keeps returning to a state it already left (A -> B -> A -> B ...) is moving
//! without getting anywhere, and the unchanged-tree count above reads every
//! such move as progress. So the digests the tree passes through are kept, and
//! [`OSCILLATION_RETURNS`] returns in a row to an already-left state, with no
//! new state reached in between, end the session too.
//!
//! Only inside a workflow run: an interactive agent is reminded, never cut.
//! The session ends with [`archon_tools::NO_PROGRESS_STOP_MARKER`], which the
//! write layer treats as a host interruption: the partial work is kept, and
//! the stop is terminal for that unit. It is routed for remediation, never
//! re-asked in-run: re-asking the same work feeds the same loop.

use std::collections::VecDeque;
use std::path::Path;

/// Tool rounds, after the reminder, that the tree may stay unchanged while the
/// answers keep repeating. One full novelty window: long enough to act on the
/// reminder, short enough that a spinning agent is not left for hours.
pub(super) const STALL_ROUNDS: u32 = 8;

/// Returns in a row to a tree state the session had already left, with no new
/// state between them, that end it. One return is a reverted experiment and two
/// a second try; a third back-and-forth with nothing new is a loop.
pub(super) const OSCILLATION_RETURNS: u32 = 3;

/// Distinct tree states remembered for the oscillation check: enough for a
/// cycle through a few states, not so many that ordinary work over hours
/// re-enters a state it left long ago and is called a loop.
const OSCILLATION_MEMORY: usize = 4;

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
    oscillation: Oscillation,
}

/// The tree states a session passed through, for the oscillation verdict.
#[derive(Debug, Default)]
struct Oscillation {
    current: Option<u64>,
    /// States the tree has left, most recent last, without repeats.
    left: VecDeque<u64>,
    returns: u32,
}

impl Oscillation {
    /// `true` once the tree made [`OSCILLATION_RETURNS`] returns in a row.
    /// A directory git cannot digest is never judged: there is no state to
    /// compare, and the write count alone cannot tell a revert from an edit.
    fn observe(&mut self, tree: Option<u64>) -> bool {
        let Some(tree) = tree else {
            return false;
        };
        let Some(current) = self.current.replace(tree) else {
            return false;
        };
        if current == tree {
            return false;
        }
        if self.left.contains(&tree) {
            self.returns += 1;
        } else {
            self.returns = 0;
        }
        self.left.retain(|state| *state != current);
        if self.left.len() == OSCILLATION_MEMORY {
            self.left.pop_front();
        }
        self.left.push_back(current);
        self.returns >= OSCILLATION_RETURNS
    }
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
        let agent = ctx.subagent_id.as_deref().unwrap_or_default();
        let state = TreeState {
            tree: tree_digest(&ctx.working_dir).await,
            writes: archon_tools::session_progress::writes(agent),
        };
        if self.oscillation.observe(state.tree) {
            return Some(archon_tools::oscillation_stop_message(OSCILLATION_RETURNS));
        }
        let key = archon_tools::repeat_tool_guard::ChainKey::of(ctx);
        if !archon_tools::repeat_tool_guard::REPEAT_TOOL_CHAINS.novelty_stalled(&key) {
            self.baseline = None;
            self.unchanged_rounds = 0;
            return None;
        }
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
            // Read every round now (the oscillation check), so it must never
            // take the index lock an agent's own git command may need.
            tokio::process::Command::new("git")
                .arg("--no-optional-locks")
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

    /// Issue-213 C2: A -> B -> A -> B -> A is three returns to a state the
    /// tree had left, with nothing new reached: a loop, whatever the answers.
    #[test]
    fn a_tree_that_keeps_returning_to_a_state_it_left_is_an_oscillation() {
        let mut osc = Oscillation::default();
        let fired: Vec<bool> = [1, 2, 1, 2, 1].map(|s| osc.observe(Some(s))).into();
        assert_eq!(fired, [false, false, false, false, true]);
        // A cycle through three states is the same verdict.
        let mut osc = Oscillation::default();
        let fired: Vec<bool> = [1, 2, 3, 1, 2, 3].map(|s| osc.observe(Some(s))).into();
        assert_eq!(fired, [false, false, false, false, false, true]);
    }

    /// One reverted experiment, a second try, unchanged rounds between moves,
    /// and any new state reached between returns are all ordinary work.
    #[test]
    fn a_revert_or_a_new_state_between_returns_is_not_an_oscillation() {
        let mut osc = Oscillation::default();
        for state in [1, 1, 2, 2, 2, 1, 1, 2, 3, 2, 3, 4, 5] {
            assert!(!osc.observe(Some(state)), "fired at {state}");
        }
        let mut osc = Oscillation::default();
        for _ in 0..10 {
            assert!(!osc.observe(None));
        }
    }

    /// The real digest: a file toggled between two contents puts the tree in
    /// the same two states, and the third return is the stop.
    #[tokio::test]
    async fn a_file_toggled_back_and_forth_trips_on_the_real_tree_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .expect("git");
            assert!(out.status.success(), "{args:?}: {out:?}");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "a\n").expect("write");
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        let mut osc = Oscillation::default();
        let mut fired = Vec::new();
        for content in ["a\n", "b\n", "a\n", "b\n", "a\n"] {
            std::fs::write(dir.path().join("f.txt"), content).expect("write");
            fired.push(osc.observe(tree_digest(dir.path()).await));
        }
        assert_eq!(fired, [false, false, false, false, true]);
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
