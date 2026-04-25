//! TASK-AGS-103: Consumer-side back-pressure for the TUI render loop.
//!
//! The agent event channel is unbounded (TASK-AGS-102) so producers can
//! never block. Back-pressure is enforced at the *consumer* by bounding
//! the in-TUI buffer and coalescing/dropping Progress events when the
//! render loop falls behind, while preserving every State event.
//!
//! Policy:
//! - SOFT_CAP = 1_000: once exceeded, begin dropping oldest Progress
//!   events. State events are always retained.
//! - HARD_CAP = 10_000: absolute ceiling. Further Progress pushes drop
//!   the oldest Progress entries to make room. State events still never
//!   dropped.
//! - RENDER_EVENT_BUDGET = 10_000: maximum events drained per frame
//!   tick so a burst cannot starve the UI redraw.

use std::collections::VecDeque;

use archon_core::agent::AgentEvent;

/// Soft cap — start shedding Progress beyond this size.
pub const SOFT_CAP: usize = 1_000;
/// Hard cap — absolute buffer ceiling. Progress dropped first.
pub const HARD_CAP: usize = 10_000;
/// Maximum events drained per render tick.
pub const RENDER_EVENT_BUDGET: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// High-value state transitions — never dropped.
    State,
    /// Incremental streaming output — droppable under overflow.
    Progress,
}

/// Classify an [`AgentEvent`] by shedding priority.
pub fn priority(ev: &AgentEvent) -> Priority {
    match ev {
        AgentEvent::TextDelta(_) | AgentEvent::ThinkingDelta(_) => Priority::Progress,
        AgentEvent::UserPromptReady
        | AgentEvent::ApiCallStarted { .. }
        | AgentEvent::ToolCallStarted { .. }
        | AgentEvent::ToolCallComplete { .. }
        | AgentEvent::PermissionRequired { .. }
        | AgentEvent::PermissionGranted { .. }
        | AgentEvent::PermissionDenied { .. }
        | AgentEvent::TurnComplete { .. }
        | AgentEvent::Error(_)
        | AgentEvent::CompactionTriggered
        | AgentEvent::SessionComplete
        | AgentEvent::AskUser { .. }
        | AgentEvent::MessageSent { .. } => Priority::State,
    }
}

/// FIFO event buffer with drop-oldest-Progress back-pressure.
pub struct EventCoalescer {
    buffer: VecDeque<AgentEvent>,
    soft_cap: usize,
    hard_cap: usize,
}

impl EventCoalescer {
    pub fn new(soft_cap: usize, hard_cap: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(hard_cap),
            soft_cap,
            hard_cap,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(SOFT_CAP, HARD_CAP)
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Push an event, shedding oldest Progress if the hard cap is exceeded.
    pub fn push(&mut self, event: AgentEvent) {
        self.buffer.push_back(event);
        while self.buffer.len() > self.hard_cap {
            if !self.drop_oldest_progress() {
                // Buffer is all State events — cannot shed further.
                break;
            }
        }
        // Soft cap: start shedding early once past the soft threshold, but
        // only if the oldest queued event is a droppable Progress. This
        // keeps steady-state latency bounded during slow bursts without
        // touching State events.
        while self.buffer.len() > self.soft_cap {
            let front_is_progress = self
                .buffer
                .front()
                .map(|e| priority(e) == Priority::Progress)
                .unwrap_or(false);
            if !front_is_progress {
                break;
            }
            self.buffer.pop_front();
        }
    }

    pub fn pop(&mut self) -> Option<AgentEvent> {
        self.buffer.pop_front()
    }

    /// Drop the oldest Progress event in the buffer. Returns true if one
    /// was found and removed.
    fn drop_oldest_progress(&mut self) -> bool {
        if let Some(idx) = self
            .buffer
            .iter()
            .position(|e| priority(e) == Priority::Progress)
        {
            self.buffer.remove(idx);
            true
        } else {
            false
        }
    }
}
