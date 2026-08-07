//! The port through which a workflow run reads its task board at the drain.
//!
//! The board itself is `archon_memory::board` — `BoardAccess` over a CozoDB
//! relation, reachable directly or across the memory server. This crate does
//! not depend on `archon-memory` and should not start: doing it for one
//! read at one barrier would put CozoDB, fastembed and a blocking HTTP client
//! into the dependency graph of every consumer of the workflow runtime. So the
//! direction is inverted the same way [`crate::llm_client_port`] inverts the
//! LLM and [`crate::lifecycle_host_port`] inverts the script host: the drain
//! declares the one thing it asks of a board, and the composition root that
//! already holds a `BoardAccess` supplies it.
//!
//! One method, deliberately. The drain gate is a READ at a barrier and nothing
//! else — it never claims, resolves, or declines an item. Anything that mutates
//! the board is an agent's job, done through the board's own tooling while the
//! run is still in flight.

use crate::error::WorkflowResult;

/// What a board item is for. Mirrors `archon_memory::board::BoardItemKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrainItemKind {
    Issue,
    Note,
}

/// Where an item sits in its lifecycle. Mirrors
/// `archon_memory::board::BoardStatus`, one variant per stored status, so an
/// adapter maps rather than interprets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrainStatus {
    Open,
    Claimed,
    InReview,
    GapsRemain,
    Resolved,
    Declined,
    Promoted,
    Escalated,
}

impl DrainStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Claimed => "claimed",
            Self::InReview => "in_review",
            Self::GapsRemain => "gaps_remain",
            Self::Resolved => "resolved",
            Self::Declined => "declined",
            Self::Promoted => "promoted",
            Self::Escalated => "escalated",
        }
    }
}

/// One board row, projected to the fields the drain gate judges.
///
/// A projection rather than the board's own `BoardItem` because the gate reads
/// four of its twelve fields, and naming that type here is the dependency edge
/// this port exists to avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainItem {
    pub id: String,
    pub title: String,
    pub kind: DrainItemKind,
    pub status: DrainStatus,
    /// Why the item was declined.
    ///
    /// The `board_items` relation has no column for this today, so the adapter
    /// supplies whatever the decliner actually recorded. The gate's rule does
    /// not depend on where it is stored: a `Declined` item arriving without a
    /// reason fails the drain exactly as an open one does. Declining is the
    /// only drain outcome that closes an item on nothing but an assertion, so
    /// it is the one that has to carry a justification.
    pub decline_reason: Option<String>,
}

/// The board, as the drain gate needs it.
pub trait WorkflowBoardPort: Send + Sync {
    /// Every item owned by `run_id`, in any status.
    ///
    /// All of them, not just the open ones: the gate reports what it inspected
    /// and how each item ended, and a port that pre-filtered would make an
    /// empty board and a fully drained board indistinguishable in the record.
    fn drain_items_for_run(&self, run_id: &str) -> WorkflowResult<Vec<DrainItem>>;
}
