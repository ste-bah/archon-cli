//! Shutdown draining for mirrored board items.
//!
//! A distinct concern from managing tasks: nothing here is part of the
//! `TaskManager` API, and the only thing it reads is
//! [`TaskManager::pending_board_items`](super::TaskManager::pending_board_items).
//! It exists solely for the moment the process is about to end.

use std::time::{Duration, Instant};

use crate::board::{DelegatedOutcome, close_delegated_task};

use super::TASK_MANAGER;

/// How long [`drain_board_items`] waits for in-flight work before giving up.
///
/// Long enough for a subagent that has already finished to run its completion
/// tail — the case this exists for is a tail losing a race with `exit`, which is
/// microseconds of work, not seconds. Short enough that a genuinely stuck agent
/// cannot hold the process open: whatever is left is released rather than
/// waited on.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the drain re-checks. Small, because the expected wait is one tick.
const DRAIN_POLL: Duration = Duration::from_millis(50);

/// Close out mirrored board items before the process exits.
///
/// `handle_print_mode_if_requested` ends with `std::process::exit`, which runs
/// no destructors and does not let detached tokio tasks finish. A background
/// subagent dispatched through `TaskCreate` records its terminal status from
/// exactly such a task, and that is what closes its board item — so a subagent
/// finishing in the same instant the run ends leaves an item claimed by an agent
/// that no longer exists, forever. Observed live: a retried task wrote its file
/// at 18:22, the run exited at 18:22, and the item was still `claimed` twenty
/// minutes later with the work sitting completed on disk.
///
/// Two phases, because the two cases want opposite things. **Wait first**: an
/// agent that has already finished only needs its tail scheduled, so a short
/// poll lets the normal path close the item with its true outcome. **Then
/// release**: anything still unfinished is genuinely unfinished, and returning
/// it to `open` with no holder says so — the same disposition the lease sweep
/// applies to a claim whose holder is gone, which is what this process is about
/// to become.
///
/// Releasing rather than resolving is deliberate. At exit this cannot know
/// whether the work succeeded, and marking it resolved would assert something
/// unverified; `open` is the honest answer and is the one that gets the item
/// picked up again.
pub async fn drain_board_items() {
    drain_board_items_within(DRAIN_TIMEOUT).await;
}

/// [`drain_board_items`] with an explicit budget, so a test can exercise the
/// give-up path without waiting the production timeout.
async fn drain_board_items_within(budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let pending = TASK_MANAGER.pending_board_items();
        if pending.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                count = pending.len(),
                "board items still claimed at exit; releasing them so they are not \
                 held by an agent this process is about to take with it"
            );
            for (task_id, item_id) in pending {
                tracing::debug!(%task_id, %item_id, "releasing board claim at exit");
                close_delegated_task(&item_id, DelegatedOutcome::Stopped);
            }
            return;
        }
        tokio::time::sleep(DRAIN_POLL).await;
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;

    use crate::task_manager::{TaskManager, TaskStatus};

    /// The drain must not delay an exit that has nothing to wait for. Every run
    /// pays this cost, including the overwhelming majority that dispatched no
    /// background subagent at all.
    #[tokio::test]
    async fn nothing_pending_returns_without_waiting() {
        let started = Instant::now();
        drain_board_items_within(Duration::from_secs(5)).await;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "an empty drain must return immediately, not burn the budget"
        );
    }

    /// A task with no mirrored item is not something to wait for: the board has
    /// nothing recorded for it, so there is nothing to close.
    #[tokio::test]
    async fn a_task_without_a_board_item_is_not_pending() {
        let mgr = TaskManager::new();
        let id = mgr.create_task("no subagent, no mirror");
        mgr.set_status(&id, TaskStatus::Running).unwrap();
        assert!(
            mgr.pending_board_items().is_empty(),
            "only mirrored tasks can strand a board item"
        );
    }

    /// The case this exists for: the completion tail lands during the drain, so
    /// the item closes by the normal path with its true outcome and the drain
    /// stops waiting.
    #[tokio::test]
    async fn a_task_that_finishes_during_the_drain_stops_being_pending() {
        let mgr = TaskManager::new();
        let id = mgr.create_task("dispatched in the background");
        mgr.set_board_item_id(&id, "agent-uuid-1");
        mgr.set_status(&id, TaskStatus::Running).unwrap();
        assert_eq!(mgr.pending_board_items().len(), 1);

        // What the detached completion task does when it finally gets scheduled.
        mgr.set_status(&id, TaskStatus::Completed).unwrap();
        assert!(
            mgr.pending_board_items().is_empty(),
            "a terminal task is no longer something the drain waits on"
        );
    }

    /// A stuck agent must not hold the process open. The budget expires, the
    /// straggler is released rather than waited on, and the drain returns.
    #[tokio::test]
    async fn a_task_that_never_finishes_gives_up_at_the_budget() {
        // Uses the global manager because `drain_board_items_within` reads it;
        // the task is left Running for the whole call on purpose.
        let id = TASK_MANAGER.create_task("an agent that never reports back");
        TASK_MANAGER.set_board_item_id(&id, "agent-uuid-stuck");
        TASK_MANAGER.set_status(&id, TaskStatus::Running).unwrap();

        let started = Instant::now();
        drain_board_items_within(Duration::from_millis(200)).await;
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(200),
            "the drain must actually wait before giving up, or the tail never lands"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the drain must give up at its budget rather than block the exit"
        );

        // Leave the global clean for any other test in this binary.
        TASK_MANAGER.set_status(&id, TaskStatus::Stopped).unwrap();
    }
}
