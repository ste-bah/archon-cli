//! Agent task board — run-scoped handoffs between subagents.
//!
//! Board items live in their own `board_items` relation, NOT in `memories`,
//! and the difference is not cosmetic. `memories` has a fixed twelve-column
//! schema, so board state would have to be encoded into the tag vector against
//! a ceiling of sixteen non-trend tags that `graph/crud_importance.rs` asserts
//! in Datalog. Worse, `update_memory` replaces that whole vector
//! last-writer-wins with no compare-and-set, so two agents claiming one item
//! would both believe they won. A board is also polled, and a `memory_type`
//! filter is a full relation scan.
//!
//! The other half of the argument is decay. Everything in `memories` is subject
//! to the garden: importance decay, staleness pruning, overflow pruning, and
//! semantic merging. An item recording work that must happen cannot be allowed
//! to fade because nobody read it for thirty days. Its own relation is what
//! makes that structurally impossible rather than a rule someone has to
//! remember — the precedent being `PersonalitySnapshot`, which serialised JSON
//! into `content` and then needed `drop_state_snapshots` filters retrofitted at
//! three separate read paths.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::MemoryError;

mod claim;
mod crud;
mod history;
mod rows;

#[cfg(test)]
#[path = "board/board_tests.rs"]
mod board_tests;
#[cfg(test)]
#[path = "board/history_tests.rs"]
mod history_tests;

/// What a board item is for.
///
/// Kept apart because the lifecycles differ: an issue outlives the run that
/// raised it and must resolve, be promoted, or be declined; a note is context
/// for whoever next touches the area and dies with the run. Conflated, a board
/// fills up with "looked at X, seemed fine" and the drain gate becomes noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoardItemKind {
    Issue,
    Note,
}

impl fmt::Display for BoardItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Issue => "issue",
            Self::Note => "note",
        })
    }
}

impl BoardItemKind {
    /// Parse from a stored string. Returns `None` for unknown values.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "issue" => Some(Self::Issue),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// Where an item sits in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoardStatus {
    Open,
    Claimed,
    InReview,
    GapsRemain,
    Resolved,
    Declined,
    Promoted,
    Escalated,
}

impl fmt::Display for BoardStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Open => "open",
            Self::Claimed => "claimed",
            Self::InReview => "in_review",
            Self::GapsRemain => "gaps_remain",
            Self::Resolved => "resolved",
            Self::Declined => "declined",
            Self::Promoted => "promoted",
            Self::Escalated => "escalated",
        })
    }
}

impl BoardStatus {
    /// Parse from a stored string. Returns `None` for unknown values.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "claimed" => Some(Self::Claimed),
            "in_review" => Some(Self::InReview),
            "gaps_remain" => Some(Self::GapsRemain),
            "resolved" => Some(Self::Resolved),
            "declined" => Some(Self::Declined),
            "promoted" => Some(Self::Promoted),
            "escalated" => Some(Self::Escalated),
            _ => None,
        }
    }
}

/// A single row of the task board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardItem {
    pub id: String,
    /// The run that owns this item. Every subagent inherits the parent's, and
    /// the drain gate is defined over exactly this partition.
    pub run_id: String,
    pub kind: BoardItemKind,
    pub status: BoardStatus,
    pub title: String,
    /// File references and what was observed. Required — see
    /// [`MemoryGraph::create_board_item`](crate::MemoryGraph::create_board_item).
    pub evidence: String,
    /// What "done" means. Rewritten by the parent on re-scope, because guidance
    /// delivered as a message leaves the reviewer verifying the old criteria.
    pub acceptance: String,
    pub raised_by: String,
    /// The agent currently holding the item, if any.
    pub claimed_by: Option<String>,
    /// Attempt counter, 0-based. Lives on the item rather than in the
    /// implementer so that an agent dying does not reset the loop.
    pub round: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Why the item was declined, for an item that was.
    ///
    /// Derived on read from the transition history rather than stored on the
    /// row — see [`MemoryGraph::init_board_history_schema`] for why the column
    /// could not exist. It is carried here anyway because the drain gate judges
    /// a declined item on this field and reads the whole run at once: a shape
    /// that made the caller fetch it separately would be one round trip per
    /// declined item across the memory socket.
    ///
    /// `None` on anything not `Declined`, and — until the storage layer refused
    /// it — `None` was also how a declined item with no justification looked.
    /// It cannot be written that way any more; see
    /// [`MemoryGraph::decline_board_item`].
    #[serde(default)]
    pub decline_reason: Option<String>,
}

/// One recorded transition in an item's life.
///
/// The escalation ladder needs to know what an item has already been through —
/// which round left gaps, what a reviewer said, why a decline was refused — and
/// that is a sequence, not a value. A column could only hold it as serialised
/// JSON inside a string, which is the `PersonalitySnapshot` mistake this
/// module's header opens by naming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardEvent {
    pub item_id: String,
    /// Per-item, 0-based, allocated inside the transition's own transaction.
    pub seq: u32,
    pub at: DateTime<Utc>,
    /// Copied from the item so a run can read its whole history in one query.
    pub run_id: String,
    pub from_status: BoardStatus,
    pub to_status: BoardStatus,
    /// The item's attempt counter when the transition happened.
    pub round: u32,
    /// Who held the item at the time, if anyone.
    pub actor: Option<String>,
    /// What the transition recorded. Required for a decline, empty otherwise.
    pub note: String,
}

/// The fields a caller supplies when raising an item.
///
/// A struct rather than eight positional arguments, so that adding a field
/// later cannot silently transpose two `&str`s at a call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBoardItem {
    /// Caller-selected id, or `None` to mint a UUID.
    #[serde(default)]
    pub id: Option<String>,
    pub run_id: String,
    pub kind: BoardItemKind,
    pub title: String,
    pub evidence: String,
    pub acceptance: String,
    pub raised_by: String,
}

/// One run that has items on the board.
///
/// Every other read here starts from a `run_id` the caller already has, because
/// every writer does: an agent inherits its parent's. A reader that arrived
/// from outside the run — a dashboard, an operator asking what is outstanding —
/// has no such handle, and before this existed there was no way to obtain one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardRunSummary {
    pub run_id: String,
    /// How many items sit in each status, keyed by the same lowercase names the
    /// `status` column and the RPC surface use. Statuses with no items are
    /// absent rather than present as zero, so a caller reads presence directly.
    pub counts: BTreeMap<String, u32>,
    pub total: u32,
    /// The newest `updated_at` across the run's items. What "most recently
    /// touched" means for a run, and the key the list is ordered on.
    pub last_updated_at: DateTime<Utc>,
}

/// The outcome of a conditional board write.
///
/// `applied` comes from the same database transaction that decided it, not from
/// a preflight read — which is the entire point of the operations that return
/// this type. `item` is the authoritative row afterwards either way, so a loser
/// can see who actually holds the item without a second round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardUpdate {
    pub applied: bool,
    pub item: BoardItem,
}

/// Board operations, available directly or across the memory server.
///
/// Deliberately separate from [`MemoryTrait`](crate::MemoryTrait): the board is
/// not a memory, and the seventeen existing `MemoryTrait` implementations
/// (mocks and stubs across four crates) have no business growing board methods
/// they would only stub out.
pub trait BoardAccess: Send + Sync {
    fn create_board_item(&self, item: &NewBoardItem) -> Result<BoardItem, MemoryError>;

    fn get_board_item(&self, id: &str) -> Result<BoardItem, MemoryError>;

    /// Every run with items on the board, most recently touched first.
    ///
    /// The one board read that takes no `run_id`, and therefore the only entry
    /// point for a reader that did not raise anything itself.
    fn list_board_runs(&self) -> Result<Vec<BoardRunSummary>, MemoryError>;

    /// Items owned by `run_id`, oldest first. An empty `statuses` means all.
    fn list_board_items_by_run(
        &self,
        run_id: &str,
        statuses: &[BoardStatus],
    ) -> Result<Vec<BoardItem>, MemoryError>;

    /// Take ownership of an unclaimed item.
    ///
    /// `applied` is true only for the caller that actually took it.
    fn claim_board_item(&self, id: &str, agent_id: &str) -> Result<BoardUpdate, MemoryError>;

    /// Give back a claim. `applied` is false if the item was not claimed.
    fn release_board_claim(&self, id: &str) -> Result<BoardUpdate, MemoryError>;

    /// Move an item between statuses, conditional on `from` still holding.
    ///
    /// Refuses `to == Declined`: that transition has to carry a reason, so it
    /// has its own method rather than an optional argument here.
    fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError>;

    /// Close an item as `declined`, recording why.
    ///
    /// Separate from [`Self::set_board_item_status`], and `reason` is a `&str`
    /// rather than an `Option`, so that "declined without a reason" is not a
    /// call anyone can make. Declining is the only ending that closes an item on
    /// an assertion alone; the drain gate refuses one with nothing behind it,
    /// and a gate is the wrong and last place for that rule to live.
    fn decline_board_item(
        &self,
        id: &str,
        from: BoardStatus,
        reason: &str,
    ) -> Result<BoardUpdate, MemoryError>;

    /// Every recorded transition for one item, oldest first.
    fn board_item_history(&self, id: &str) -> Result<Vec<BoardEvent>, MemoryError>;

    /// The run's transitions across all of its items, newest first.
    ///
    /// The counterpart to [`Self::board_item_history`] for a reader who has a
    /// run but not an item: asking "what just happened here" per item means one
    /// query per row, and a poller would pay that on every tick.
    ///
    /// Truncated to the newest [`RUN_ACTIVITY_LIMIT`] transitions. A long-lived
    /// run accumulates transitions without bound while the question this answers
    /// is always about the recent end, so the cap is in the operation rather
    /// than left to each caller to remember.
    fn board_run_activity(&self, run_id: &str) -> Result<Vec<BoardEvent>, MemoryError>;
}

/// The most transitions [`BoardAccess::board_run_activity`] will ever return.
///
/// A constant rather than a caller-supplied limit: the cap exists to bound what
/// crosses the RPC surface and what a dashboard renders, and a parameter would
/// let either side ask for the unbounded answer the cap is here to prevent.
pub const RUN_ACTIVITY_LIMIT: usize = 200;
