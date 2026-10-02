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
//! as a digest of `git status`, `git diff HEAD` and the contents of untracked
//! files in the session's working directory, together with the write count
//! the write tools recorded, so a directory that is not a repository still
//! shows its writes.
//!
//! A second verdict needs no repeated answers at all: OSCILLATION. A tree that
//! keeps returning to a state it already left (A -> B -> A -> B ...) is moving
//! without getting anywhere, and the unchanged-tree count above reads every
//! such move as progress. So the digests the tree passes through are kept, and
//! [`OSCILLATION_RETURNS`] returns in a row to an already-left state end the
//! session too. Only a write-capable session is judged, only on rounds where
//! it wrote or called a tool that may write, and a return counts only when the session
//! wrote no new path in that round (or its answers are stalled as well): a
//! session growing a new file while some regenerated file flips back is
//! working, not looping.
//!
//! The tree is probed only on a round that could have changed it — one with a
//! write, or a call to any tool not known to be read-only — and on every round
//! of a stalled session, so a reading round of a working session costs no git
//! at all. A return also does not count when the round wrote somewhere the
//! digest cannot see (an ignored or out-of-tree path).
//!
//! Only inside a workflow run: an interactive agent is reminded, never cut.
//! The session ends with [`archon_tools::NO_PROGRESS_STOP_MARKER`], which the
//! write layer treats as a host interruption: the partial work is kept, and
//! the stop is terminal for that unit. It is routed for remediation, never
//! re-asked in-run: re-asking the same work feeds the same loop.

use std::collections::VecDeque;

#[path = "progress_stop_tree.rs"]
mod tree;
#[path = "progress_stop_untracked.rs"]
mod untracked;
use tree::tree_digest;

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

/// The tools known to change nothing on disk. Every other tool — a shell, a
/// child agent, an MCP tool, a terminal — may change the tree without the
/// write tools seeing it, so a round that called one is probed.
const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "WebFetch",
    "WebSearch",
    "ToolSearch",
];

/// What one tool round did that could have changed the working tree, beyond
/// the writes the write tools record themselves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RoundActivity {
    pub(super) may_change_tree: bool,
}

impl RoundActivity {
    pub(super) fn of<'a>(tools: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            may_change_tree: tools
                .into_iter()
                .any(|name| !READ_ONLY_TOOLS.contains(&name)),
        }
    }
}

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
    /// The last tree digest probed, reused on a round that could not move it.
    last_tree: Option<Option<u64>>,
    last_writes: u64,
    last_touched: usize,
    /// Tree probes run, so a test can see a reading round cost none.
    probes: u32,
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
    /// `true` once the tree made [`OSCILLATION_RETURNS`] counted returns in a
    /// row. `counts` is false for a round that also wrote a new path: that is
    /// progress, and it breaks the run. A directory git cannot digest is
    /// never judged: there is no state to compare.
    fn observe(&mut self, tree: Option<u64>, counts: bool) -> bool {
        let Some(tree) = tree else {
            return false;
        };
        let Some(current) = self.current.replace(tree) else {
            return false;
        };
        if current == tree {
            return false;
        }
        if counts && self.left.contains(&tree) {
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

/// A session whose own writes the oscillation verdict judges: write-capable
/// under its workflow guard. Read-only and verifier sessions are exempt.
fn write_capable(ctx: &archon_tools::tool::ToolContext) -> bool {
    ctx.workflow_read_guard.as_ref().is_some_and(|guard| {
        guard.mode() == archon_tools::workflow_read_guard::GuardMode::WriteCapable
    })
}

impl ProgressStop {
    /// Observe the end of one tool round. `Some(reason)` ends the session.
    pub(super) async fn after_round(
        &mut self,
        ctx: &archon_tools::tool::ToolContext,
        round: RoundActivity,
    ) -> Option<String> {
        if ctx.run_store.is_none() && ctx.workflow_read_guard.is_none() {
            return None;
        }
        let agent = ctx.subagent_id.as_deref().unwrap_or_default();
        let writes = archon_tools::session_progress::writes(agent);
        let touched = archon_tools::session_progress::touched_paths(agent);
        let acted = writes != self.last_writes || round.may_change_tree;
        let key = archon_tools::repeat_tool_guard::ChainKey::of(ctx);
        let stalled = archon_tools::repeat_tool_guard::REPEAT_TOOL_CHAINS.novelty_stalled(&key);
        // A stalled session is probed every round: whatever moved its tree
        // (another agent, a tool the list does not know) must count as
        // movement before it is cut for standing still.
        let tree = match self.last_tree {
            Some(tree) if !acted && !stalled => tree,
            None if !acted && !stalled => None,
            _ => {
                self.probes += 1;
                let tree = tree_digest(&ctx.working_dir).await;
                self.last_tree = Some(tree);
                tree
            }
        };
        let new_path = touched > self.last_touched;
        let unseen = writes != self.last_writes && self.wrote_unseen(ctx, agent).await;
        self.last_writes = writes;
        self.last_touched = touched;
        let counts = (!new_path || stalled) && !unseen;
        if acted && write_capable(ctx) && self.oscillation.observe(tree, counts) {
            return Some(archon_tools::oscillation_stop_message(OSCILLATION_RETURNS));
        }
        if !stalled {
            self.baseline = None;
            self.unchanged_rounds = 0;
            return None;
        }
        self.observe(TreeState { tree, writes })?;
        Some(archon_tools::no_progress_stop_message(
            self.unchanged_rounds,
        ))
    }

    /// Whether this round wrote somewhere the digest cannot see (outside the
    /// working tree, or ignored by it): a return to an old digest then says
    /// nothing about whether the session is going anywhere.
    async fn wrote_unseen(&self, ctx: &archon_tools::tool::ToolContext, agent: &str) -> bool {
        match archon_tools::session_progress::writes_since(agent, self.last_writes) {
            Some(paths) => tree::any_invisible(&ctx.working_dir, &paths).await,
            // More writes than are kept: assume one was out of sight.
            None => true,
        }
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

#[cfg(test)]
#[path = "progress_stop_tests.rs"]
mod tests;
