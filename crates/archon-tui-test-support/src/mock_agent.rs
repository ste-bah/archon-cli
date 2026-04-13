//! Mock agent + fake AgentEvent source for phase-1/2/4 tests.
//!
//! Decoupled from the real `archon_tui::app::AgentEvent` type — uses a local
//! [`MockEventKind`] enum so this crate does not gain a dep on the production
//! TUI crate. Phase-1 tasks provide a thin adapter mapping `MockEventKind` to
//! the real event type at their call sites.
//!
//! See TASK-TUI-008 spec for usage contracts.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// One scriptable mock event kind. Deliberately decoupled from the production
/// `AgentEvent` so this crate does not depend on `archon-tui`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockEventKind {
    ToolCall(String),
    MessageDelta(String),
    ThoughtChunk(String),
    ToolResult(String),
    Finish,
}

/// Scripted sequence of events a [`MockAgent`] will emit.
#[derive(Debug, Clone, Default)]
pub struct EventScript {
    events: Vec<MockEventKind>,
    interval: Duration,
    total: usize,
}

impl EventScript {
    /// Create an empty script with zero interval.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a `ToolCall(name)` event.
    pub fn tool_call(mut self, name: impl Into<String>) -> Self {
        self.events.push(MockEventKind::ToolCall(name.into()));
        self.total = self.events.len();
        self
    }

    /// Append a `MessageDelta(text)` event.
    pub fn message_delta(mut self, text: impl Into<String>) -> Self {
        self.events.push(MockEventKind::MessageDelta(text.into()));
        self.total = self.events.len();
        self
    }

    /// Append a `ThoughtChunk(text)` event.
    pub fn thought_chunk(mut self, text: impl Into<String>) -> Self {
        self.events.push(MockEventKind::ThoughtChunk(text.into()));
        self.total = self.events.len();
        self
    }

    /// Append a `ToolResult(text)` event.
    pub fn tool_result(mut self, text: impl Into<String>) -> Self {
        self.events.push(MockEventKind::ToolResult(text.into()));
        self.total = self.events.len();
        self
    }

    /// Append a `Finish` event.
    pub fn finish(mut self) -> Self {
        self.events.push(MockEventKind::Finish);
        self.total = self.events.len();
        self
    }

    /// Append `count` copies of the same kind — handy for load tests.
    pub fn burst_of(mut self, count: usize, kind: MockEventKind) -> Self {
        for _ in 0..count {
            self.events.push(kind.clone());
        }
        self.total = self.events.len();
        self
    }

    /// Set the per-event delay. With a paused tokio clock this becomes
    /// a virtual wait.
    pub fn sleep(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Current scripted event count.
    pub fn total(&self) -> usize {
        self.total
    }
}

/// Return value of a completed or cancelled [`MockAgent::run`].
#[derive(Debug, Clone)]
pub struct MockAgentReport {
    pub id: String,
    pub events_sent: usize,
    pub events_dropped: usize,
    pub elapsed: Duration,
    pub cancelled: bool,
}

/// Error returned from a failed [`MockEventSink::send`]. Mirrors the shape
/// of tokio channel send errors without binding to any specific channel flavour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError;

/// Sink abstraction so `MockAgent` can push into either a bounded
/// `mpsc::Sender<MockEventKind>` or an `mpsc::UnboundedSender<MockEventKind>`.
pub trait MockEventSink: Send + Sync + 'static {
    /// Try to send one event. Bounded sinks return `Err(SendError)` when full.
    fn send(&self, ev: MockEventKind) -> Result<(), SendError>;
}

impl MockEventSink for mpsc::UnboundedSender<MockEventKind> {
    fn send(&self, ev: MockEventKind) -> Result<(), SendError> {
        mpsc::UnboundedSender::send(self, ev).map_err(|_| SendError)
    }
}

impl MockEventSink for mpsc::Sender<MockEventKind> {
    fn send(&self, ev: MockEventKind) -> Result<(), SendError> {
        self.try_send(ev).map_err(|_| SendError)
    }
}

/// Fake agent that replays a scripted sequence of events into a sink.
pub struct MockAgent {
    id: String,
    script: EventScript,
    cancel: CancellationToken,
}

impl MockAgent {
    /// Construct a new mock with the given id and script.
    pub fn new(id: impl Into<String>, script: EventScript) -> Self {
        Self {
            id: id.into(),
            script,
            cancel: CancellationToken::new(),
        }
    }

    /// Trigger cooperative cancellation for an in-flight `run`.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Clone the internal cancellation token so callers can cancel from a
    /// separate task without needing a reference to the owned `MockAgent`.
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Run the script against the given sink. Returns a report after either
    /// the full script is delivered or the cancellation token fires.
    pub async fn run<S>(self, sink: S) -> MockAgentReport
    where
        S: MockEventSink,
    {
        let start = tokio::time::Instant::now();
        let mut events_sent = 0usize;
        let mut events_dropped = 0usize;
        let mut cancelled = false;

        for ev in self.script.events.into_iter() {
            if self.cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            if !self.script.interval.is_zero() {
                tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => {
                        cancelled = true;
                        break;
                    }
                    _ = tokio::time::sleep(self.script.interval) => {}
                }
                if self.cancel.is_cancelled() {
                    cancelled = true;
                    break;
                }
            }
            match sink.send(ev) {
                Ok(()) => events_sent += 1,
                Err(_) => events_dropped += 1,
            }
        }

        MockAgentReport {
            id: self.id,
            events_sent,
            events_dropped,
            elapsed: tokio::time::Instant::now().saturating_duration_since(start),
            cancelled,
        }
    }
}

/// Spawn `n` mock agents each running the same scripted total, returning a
/// vector of JoinHandles.
///
/// Each agent gets id `"mock-{i}"` and a fresh script consisting of
/// `per_agent_events` `MessageDelta("tick")` events with zero interval.
pub fn spawn_n_mock_agents<S>(
    n: usize,
    per_agent_events: usize,
    tx: S,
) -> Vec<JoinHandle<MockAgentReport>>
where
    S: MockEventSink + Clone,
{
    let counter = Arc::new(AtomicUsize::new(0));
    (0..n)
        .map(|i| {
            let _ = counter.fetch_add(1, Ordering::Relaxed);
            let id = format!("mock-{}", i);
            let script = EventScript::new().burst_of(
                per_agent_events,
                MockEventKind::MessageDelta("tick".into()),
            );
            let agent = MockAgent::new(id, script);
            let sink = tx.clone();
            tokio::spawn(async move { agent.run(sink).await })
        })
        .collect()
}
