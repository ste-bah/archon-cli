//! Layout + resize reflow helpers for the TUI event loop.
//!
//! TUI-105: `handle_resize` is the non-blocking reflow entry point
//! called by the crossterm event dispatch path whenever a SIGWINCH
//! lands as an `Event::Resize(cols, rows)`. Zero `.await`, zero
//! allocations on the hot path, zero contact with AgentDispatcher
//! state (`current_query`, `pending_queue`).
//!
//! This file exists as a separate module (rather than sitting inside
//! `task_dispatch.rs`) because `task_dispatch.rs` is already 382 lines
//! and the technical spec's "or a new `layout.rs` helper if
//! `task_dispatch.rs` would exceed 300 lines" clause applies.

use std::sync::{Mutex, OnceLock};

/// Global last-known terminal size. Written on every `handle_resize`
/// call; readable by render code that needs the current viewport
/// dimensions without threading them through every callsite.
///
/// Rationale: `Mutex<(u16, u16)>` over `AtomicU32` because cols+rows
/// are always read/written as a pair — atomic pair semantics avoid
/// the encode/decode dance and keep the test fixture trivial. The
/// mutex is uncontended in practice (resize events are rare, ~10/sec
/// peak during a drag) so Mutex overhead is noise.
static LAST_KNOWN_SIZE: OnceLock<Mutex<(u16, u16)>> = OnceLock::new();

fn size_cell() -> &'static Mutex<(u16, u16)> {
    LAST_KNOWN_SIZE.get_or_init(|| Mutex::new((0, 0)))
}

/// Outcome of a reflow invocation. `dirty` is constant-true by design:
/// every resize event forces a redraw because ratatui's layout cache
/// is invalidated by dimension changes, so lazy invalidation would
/// only add a branch with no payoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflowOutcome {
    pub cols: u16,
    pub rows: u16,
    pub dirty: bool,
}

/// Record the new terminal size and return a `ReflowOutcome` marking
/// the render as dirty.
///
/// **Non-blocking contract:** this function MUST NOT `.await`, MUST
/// NOT lock the `AgentDispatcher`, MUST NOT touch `current_query` or
/// `pending_queue`. If you add any of those, you have broken
/// TUI-105's reason for existing (AC-EVENTLOOP-03: "resize never
/// blocks").
pub fn handle_resize(cols: u16, rows: u16) -> ReflowOutcome {
    {
        let mut slot = size_cell().lock().expect("LAST_KNOWN_SIZE poisoned");
        *slot = (cols, rows);
    }
    ReflowOutcome {
        cols,
        rows,
        dirty: true,
    }
}

/// Read the last recorded size. Returns `(0, 0)` before the first
/// resize has been observed.
pub fn last_known_size() -> (u16, u16) {
    *size_cell().lock().expect("LAST_KNOWN_SIZE poisoned")
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
