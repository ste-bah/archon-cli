//! Mirror delegated tasks onto the task board.
//!
//! The board had every part built except a producer. `BoardRaise` let an agent
//! record a finding it chose to write down, and the web UI reads
//! `list_board_runs`, which lists runs *that have items* — so a session whose
//! agents never called `BoardRaise` was invisible, and every session's fan-out
//! was invisible, because spawning a subagent raised nothing. This module is
//! the missing producer: dispatching work through `TaskCreate` puts that work
//! on the board, and finishing it closes the item out.
//!
//! Three things here are deliberate.
//!
//! **The item id is the subagent id.** A delegated item is looked up again at
//! terminal status, from a different call stack, and minting a fresh uuid would
//! mean storing a second id somewhere just to find the row. The subagent id is
//! already unique, already recorded against the task, and already what
//! `claimed_by` holds — reusing it makes the lookup free. Note this is the full
//! subagent uuid, not the 8-char `TASK_MANAGER` id: those are disjoint
//! namespaces, and the 8-char id is only unique within one process.
//!
//! **A mirrored item is raised and then immediately claimed**, because that is
//! what is true — an agent is already holding it. It also buys the lease sweep
//! for free: [`release_dead_claims`](super::release_dead_claims) reopens items
//! whose holder is gone, so a subagent that dies mid-task returns its work to
//! the board instead of leaving a row that claims to be in progress forever.
//!
//! **Every failure here is soft.** A board write must never be able to stop a
//! subagent from being dispatched. Most processes that run `TaskCreate` have no
//! memory service open at all — every test registry, for one — and in those the
//! handle does not resolve and mirroring is simply skipped.

use archon_memory::board::{BoardItemKind, BoardStatus, NewBoardItem};

use super::{BoardHandle, run_id_for_session};

/// How a delegated task ended, in the terms the board cares about.
///
/// Separate from `TaskStatus` so this module does not have to grow a case every
/// time the task lifecycle gains a non-terminal state: the only distinctions
/// the board draws are "done", "needs attention", and "given back".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegatedOutcome {
    Completed,
    Failed,
    Stopped,
}

/// Put a dispatched subagent on the board, returning the item id.
///
/// `description` names the task and `instruction` is the prompt the subagent
/// was actually handed — kept separate because they are not the same thing. A
/// description is written for a human scanning a list; a prompt is the full
/// brief, and it is the prompt that tells a later reader what was really asked.
///
/// `None` means the board was unreachable or refused the write, and the caller
/// carries on regardless — see the module note on soft failure.
pub fn raise_delegated_task(
    session_id: &str,
    subagent_id: &str,
    description: &str,
    instruction: &str,
    raised_by: &str,
) -> Option<String> {
    raise_on(
        &BoardHandle::Global,
        session_id,
        subagent_id,
        description,
        instruction,
        raised_by,
    )
}

/// [`raise_delegated_task`] against an explicit handle.
///
/// Split out for the same reason [`BoardHandle::Direct`] exists at all: the
/// global is a `OnceLock`, so a test that had to install one would be
/// ordering-dependent on every other test in the binary. Production always
/// passes `Global`.
fn raise_on(
    handle: &BoardHandle,
    session_id: &str,
    subagent_id: &str,
    description: &str,
    instruction: &str,
    raised_by: &str,
) -> Option<String> {
    let board = match handle.resolve() {
        Ok(board) => board,
        // Expected whenever no memory service is open, so this is not a warning.
        Err(reason) => {
            tracing::debug!(%reason, "not mirroring delegated task to the board");
            return None;
        }
    };

    let item = NewBoardItem {
        id: Some(subagent_id.to_string()),
        run_id: run_id_for_session(session_id).to_string(),
        kind: BoardItemKind::Issue,
        title: summarise(description),
        // Storage rejects empty evidence, and rightly so — but the evidence for
        // a delegated task is not a file reference, it is the instruction the
        // agent was given. Recording it verbatim (bounded) is what lets someone
        // reading the board later see what was actually asked for, which is the
        // same job file:line references do for a finding.
        evidence: format!(
            "Delegated to subagent {subagent_id}.\n\nBrief given:\n{}",
            clamp(instruction)
        ),
        acceptance: "The subagent reports the delegated work complete.".to_string(),
        raised_by: raised_by.to_string(),
    };

    match board.create_board_item(&item) {
        Ok(stored) => {
            // Claim failure is not fatal: the item is on the board either way,
            // which is the point. It stays `open` and the sweep leaves it alone.
            if let Err(error) = board.claim_board_item(&stored.id, subagent_id) {
                tracing::debug!(%error, item = %stored.id, "mirrored item raised but not claimed");
            }
            Some(stored.id)
        }
        Err(error) => {
            tracing::debug!(%error, "failed to mirror delegated task to the board");
            None
        }
    }
}

/// Close out a mirrored item once its subagent reaches a terminal state.
///
/// The transition is a compare-and-set from `claimed`, so an item a human or
/// another agent has already moved on (resolved it by hand, escalated it) is
/// left as they left it rather than being overwritten by the runtime.
pub fn close_delegated_task(item_id: &str, outcome: DelegatedOutcome) {
    close_on(&BoardHandle::Global, item_id, outcome);
}

/// [`close_delegated_task`] against an explicit handle. See [`raise_on`].
fn close_on(handle: &BoardHandle, item_id: &str, outcome: DelegatedOutcome) {
    let Ok(board) = handle.resolve() else {
        return;
    };

    let result = match outcome {
        DelegatedOutcome::Completed => {
            board.set_board_item_status(item_id, BoardStatus::Claimed, BoardStatus::Resolved)
        }
        // Escalated rather than declined: the work was not refused on a
        // judgement, it failed, and somebody has to decide what happens next.
        DelegatedOutcome::Failed => {
            board.set_board_item_status(item_id, BoardStatus::Claimed, BoardStatus::Escalated)
        }
        // A stopped agent gave the work back. Releasing the claim returns the
        // item to `open` and clears the holder, so it reads as available.
        DelegatedOutcome::Stopped => board.release_board_claim(item_id),
    };

    if let Err(error) = result {
        tracing::debug!(%error, item = %item_id, ?outcome, "failed to close mirrored board item");
    }
}

/// The longest a mirrored title runs before it is cut.
///
/// A task description is free text and can be a whole prompt; the board list is
/// one line per item.
const TITLE_LIMIT: usize = 120;

/// The longest a mirrored prompt runs before it is cut.
const EVIDENCE_LIMIT: usize = 2000;

/// First line of `description`, bounded, for use as a title.
fn summarise(description: &str) -> String {
    let first_line = description.lines().find(|line| !line.trim().is_empty());
    match first_line {
        Some(line) => truncate_on_char_boundary(line.trim(), TITLE_LIMIT),
        // Storage does not reject an empty title the way it rejects empty
        // evidence, but a blank row on the board helps nobody.
        None => "Delegated task".to_string(),
    }
}

fn clamp(text: &str) -> String {
    truncate_on_char_boundary(text.trim(), EVIDENCE_LIMIT)
}

/// Cut to at most `limit` bytes without splitting a UTF-8 character.
///
/// Slicing by byte index would panic on any multi-byte character straddling the
/// limit, and a task description is arbitrary user text.
fn truncate_on_char_boundary(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use archon_memory::board::BoardAccess;
    use archon_memory::graph::MemoryGraph;

    use super::*;

    fn board() -> (BoardHandle, Arc<dyn BoardAccess>) {
        let access: Arc<dyn BoardAccess> =
            Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
        (BoardHandle::Direct(Arc::clone(&access)), access)
    }

    fn raise(handle: &BoardHandle, session: &str, agent: &str) -> String {
        raise_on(handle, session, agent, "Build the parser", "the brief", "parent")
            .expect("mirroring onto a reachable board succeeds")
    }

    /// The whole point: dispatching work puts a *claimed* item on the run's
    /// board, because an agent is already holding it.
    #[test]
    fn a_dispatched_task_lands_on_the_board_already_claimed() {
        let (handle, access) = board();
        let id = raise(&handle, "sess-abc", "agent-1");

        let item = access.get_board_item(&id).expect("item is on the board");
        assert_eq!(item.run_id, "sess-abc");
        assert_eq!(item.status, BoardStatus::Claimed);
        assert_eq!(item.claimed_by.as_deref(), Some("agent-1"));
        assert_eq!(item.title, "Build the parser");
        assert!(item.evidence.contains("the brief"), "the brief is recorded");
    }

    /// The item id is the subagent id, which is what lets the terminal path
    /// find the row again without storing a second id.
    #[test]
    fn the_item_id_is_the_subagent_id() {
        let (handle, _access) = board();
        assert_eq!(raise(&handle, "sess-abc", "agent-1"), "agent-1");
    }

    /// A stage session mirrors onto its *run's* partition, not its own, so a
    /// workflow's fan-out lands on one board rather than one board per stage.
    #[test]
    fn a_stage_session_mirrors_onto_the_run_partition() {
        let (handle, access) = board();
        let id = raise(&handle, "run-42-stage-implement-attempt-1", "agent-1");
        assert_eq!(access.get_board_item(&id).expect("item").run_id, "run-42");
    }

    #[test]
    fn completing_the_task_resolves_the_item() {
        let (handle, access) = board();
        let id = raise(&handle, "sess-abc", "agent-1");

        close_on(&handle, &id, DelegatedOutcome::Completed);

        let item = access.get_board_item(&id).expect("item");
        assert_eq!(item.status, BoardStatus::Resolved);
    }

    /// Failure escalates rather than resolving — the work did not get done, and
    /// a resolved item would hide that from anyone reading the board.
    #[test]
    fn failing_the_task_escalates_the_item() {
        let (handle, access) = board();
        let id = raise(&handle, "sess-abc", "agent-1");

        close_on(&handle, &id, DelegatedOutcome::Failed);

        assert_eq!(
            access.get_board_item(&id).expect("item").status,
            BoardStatus::Escalated
        );
    }

    /// A stopped agent gives the work back: the item returns to the pool with
    /// no holder, rather than sitting claimed by an agent that is gone.
    #[test]
    fn stopping_the_task_returns_the_item_to_the_pool() {
        let (handle, access) = board();
        let id = raise(&handle, "sess-abc", "agent-1");

        close_on(&handle, &id, DelegatedOutcome::Stopped);

        let item = access.get_board_item(&id).expect("item");
        assert_eq!(item.status, BoardStatus::Open);
        assert_eq!(item.claimed_by, None, "the holder is cleared");
    }

    /// Closing is a compare-and-set from `claimed`, so a verdict somebody else
    /// already recorded is not overwritten by the runtime's terminal update.
    #[test]
    fn closing_does_not_overwrite_a_verdict_already_recorded() {
        let (handle, access) = board();
        let id = raise(&handle, "sess-abc", "agent-1");
        access
            .decline_board_item(&id, BoardStatus::Claimed, "superseded by a different approach")
            .expect("declining a claimed item");

        close_on(&handle, &id, DelegatedOutcome::Completed);

        assert_eq!(
            access.get_board_item(&id).expect("item").status,
            BoardStatus::Declined,
            "the runtime must not resolve an item a human already closed"
        );
    }

    /// Two subagents in one session share a run, which is what makes the fan-out
    /// readable as a single unit of work in the UI.
    #[test]
    fn sibling_subagents_share_one_run_partition() {
        let (handle, access) = board();
        raise(&handle, "sess-abc", "agent-1");
        raise(&handle, "sess-abc", "agent-2");

        let items = access
            .list_board_items_by_run("sess-abc", &[])
            .expect("listing the run");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn title_is_the_first_non_empty_line() {
        assert_eq!(summarise("\n\n  Build the parser  \nmore\n"), "Build the parser");
    }

    #[test]
    fn blank_description_still_yields_a_title() {
        assert_eq!(summarise("   \n\n"), "Delegated task");
    }

    #[test]
    fn long_title_is_cut_to_the_limit() {
        let title = summarise(&"a".repeat(TITLE_LIMIT + 50));
        assert_eq!(title.chars().count(), TITLE_LIMIT + 1, "limit plus the ellipsis");
    }

    /// The cut must land on a character boundary — slicing bytes would panic.
    #[test]
    fn multibyte_text_at_the_limit_does_not_panic() {
        // '…' is 3 bytes, so a run of them straddles any limit not divisible by 3.
        let text = "…".repeat(TITLE_LIMIT);
        let title = summarise(&text);
        assert!(title.len() <= TITLE_LIMIT + '…'.len_utf8());
        assert!(title.ends_with('…'));
    }

    /// Mirroring is skipped, not failed, when no memory service is open.
    #[test]
    fn no_board_installed_returns_none_rather_than_erroring() {
        // Safe regardless of install order: this asserts the *absence* of a
        // panic and a `None`-or-`Some` outcome, and the global may legitimately
        // be installed by another test in the same binary.
        let raised =
            raise_delegated_task("session-1", "agent-1", "do the thing", "the brief", "parent");
        if raised.is_none() {
            // The unreachable-board path also has to be a no-op, not a panic.
            close_delegated_task("agent-1", DelegatedOutcome::Completed);
        }
    }
}
