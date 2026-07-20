//! TASK-AGS-103: Consumer-side back-pressure for the TUI render loop.
//!
//! The agent event channel is unbounded so producers never block. Back-pressure
//! is enforced at the consumer by coalescing adjacent assistant text and
//! shedding only ephemeral progress when the render loop falls behind. Text and
//! state transitions remain lossless.
//!
//! Policy:
//! - SOFT_CAP = 1_000: shed oldest ephemeral progress beyond this size.
//! - HARD_CAP = 10_000: continue shedding ephemeral progress. Lossless text and
//!   state may temporarily exceed the event-count cap.
//! - RENDER_EVENT_BUDGET = 10_000: maximum events drained per frame tick.

use std::collections::VecDeque;

use archon_core::agent::AgentEvent;

/// Soft cap — start shedding Progress beyond this size.
pub const SOFT_CAP: usize = 1_000;
/// Event-count threshold used to shed ephemeral progress first.
pub const HARD_CAP: usize = 10_000;
/// Maximum events drained per render tick.
pub const RENDER_EVENT_BUDGET: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// High-value state transitions — never dropped.
    State,
    /// User-visible assistant output — lossless and coalesced when adjacent.
    Text,
    /// Ephemeral incremental state — droppable under overflow.
    Progress,
}

/// Classify an [`AgentEvent`] by shedding priority.
pub fn priority(ev: &AgentEvent) -> Priority {
    match ev {
        AgentEvent::TextDelta(_) | AgentEvent::ThinkingDelta(_) => Priority::Text,
        AgentEvent::UserPromptReady
        | AgentEvent::ApiCallStarted { .. }
        | AgentEvent::ContextPressureUpdated { .. }
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

    /// Push an event, coalescing adjacent content and shedding only ephemeral Progress.
    pub fn push(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(text) => {
                if let Some(AgentEvent::TextDelta(previous)) = self.buffer.back_mut() {
                    previous.push_str(&text);
                } else {
                    self.buffer.push_back(AgentEvent::TextDelta(text));
                }
            }
            AgentEvent::ThinkingDelta(text) => {
                if let Some(AgentEvent::ThinkingDelta(previous)) = self.buffer.back_mut() {
                    previous.push_str(&text);
                } else {
                    self.buffer.push_back(AgentEvent::ThinkingDelta(text));
                }
            }
            event => self.buffer.push_back(event),
        }
        while self.buffer.len() > self.hard_cap {
            if !self.drop_oldest_progress() {
                // Buffer is entirely lossless — allow temporary cap overflow.
                break;
            }
        }
        while self.buffer.len() > self.soft_cap && self.drop_oldest_progress() {}
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
