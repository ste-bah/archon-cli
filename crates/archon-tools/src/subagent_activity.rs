//! Host-owned inactivity bound for one subagent session, separate from its
//! wall clock.
//!
//! The wall clock bounds how long a session may run. It cannot tell a session
//! that is working slowly from one that has stopped, so a session whose one
//! provider request stalled holds its slot until the wall clock runs out. The
//! stream idle guard does not close that gap on its own: it bounds ONE read
//! gap, then resends the identical request, and each resend is answered with a
//! fresh `message_start` that resets it. A stall that repeats on the resend
//! therefore costs the idle guard times its retry count — observed live as four
//! consecutive one-hour silences, exactly the session's four-hour wall clock.
//!
//! This clock measures activity where it happens. The runner touches it on
//! every model output event and when a request preparation (which may carry a
//! compaction summary) completes, and a tool round holds it open for as long as
//! the round is in flight. The host that installed it cuts the session once it
//! has been silent for the bound, and reports that cut under
//! [`INACTIVITY_TIMEOUT_MARKER`], a kind of its own.
//!
//! What is NOT activity: the opening `message_start` of a response, keep-alive
//! pings, and provider error events. A stalled provider sends exactly those and
//! nothing else, so counting them would let a stall look alive forever.
//!
//! A tool round in flight IS activity. A branch waiting on a slow tool is
//! working, and every tool already carries its own bound (the MCP call budget,
//! the Bash timeout); a tool with none is still bounded by the wall clock. The
//! clock resumes when the round ends, measured from the moment its results
//! returned.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

/// The prefix of the error a session ends with when the host cut it for
/// inactivity. Distinct from every wall-clock phrase ("timed out after",
/// "wall-clock timeout") so a record can say which bound fired.
pub const INACTIVITY_TIMEOUT_MARKER: &str = "subagent inactivity timeout:";

/// When a session last showed activity, and whether a tool round is running.
///
/// The clock does not run until the session's runner reports its first
/// activity. Before that the session may be queued for a subagent slot, and a
/// branch waiting for capacity has not gone silent — it has not started. The
/// wall clock still bounds a session that never starts.
#[derive(Debug)]
pub struct ActivityClock {
    last: Mutex<Option<Instant>>,
    tool_rounds: AtomicUsize,
}

impl ActivityClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            last: Mutex::new(None),
            tool_rounds: AtomicUsize::new(0),
        })
    }

    /// Record activity now; the first call starts the clock.
    pub fn touch(&self) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some(Instant::now());
        }
    }

    /// Hold the session active for as long as the returned guard lives. The
    /// guard's drop is itself activity: the round's results have returned.
    pub fn tool_round(self: &Arc<Self>) -> ToolRoundGuard {
        self.tool_rounds.fetch_add(1, Ordering::SeqCst);
        self.touch();
        ToolRoundGuard(Arc::clone(self))
    }

    /// When the current silence began, or `None` while a tool round runs or
    /// before the session has started.
    pub fn silent_since(&self) -> Option<Instant> {
        if self.tool_rounds.load(Ordering::SeqCst) > 0 {
            return None;
        }
        self.last.lock().ok().and_then(|last| *last)
    }
}

/// Keeps a tool round counted as activity; see [`ActivityClock::tool_round`].
#[derive(Debug)]
pub struct ToolRoundGuard(Arc<ActivityClock>);

impl Drop for ToolRoundGuard {
    fn drop(&mut self) {
        self.0.touch();
        self.0.tool_rounds.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Resolves once the session has been silent for at least `limit`, with the
/// silence it measured. Never resolves while a tool round is in flight, nor
/// before the session has started.
pub async fn silence_exceeding(clock: &ActivityClock, limit: Duration) -> Duration {
    loop {
        match clock.silent_since() {
            // Re-checked after a full bound: the round ending (or the session
            // starting) touches the clock, so the next pass measures from that
            // moment, not from this wake-up.
            None => tokio::time::sleep(limit).await,
            Some(since) => {
                let silent = Instant::now().saturating_duration_since(since);
                if silent >= limit {
                    return silent;
                }
                tokio::time::sleep_until(since + limit).await;
            }
        }
    }
}

/// The error text of an inactivity cut, carrying the marker.
pub fn inactivity_error_text(silent: Duration, limit: Duration) -> String {
    format!(
        "{INACTIVITY_TIMEOUT_MARKER} no model output, tool call or tool result for {}s \
         (inactivity limit {}s); the host ended the session inside its wall clock",
        silent.as_secs(),
        limit.as_secs()
    )
}

/// Does this error text carry an inactivity cut, however deeply wrapped?
pub fn is_inactivity_timeout_text(error: &str) -> bool {
    error.contains(INACTIVITY_TIMEOUT_MARKER)
}

/// A clock bound to the one session its host installed it for.
///
/// Keyed by agent id, like `subagent_session`, so it reaches exactly that
/// session's executor: a subagent the session itself spawns (through a tool)
/// has an id of its own and never feeds the parent's clock. A foreground child
/// is already covered by the parent's tool round; a backgrounded one must not
/// keep a stalled parent looking alive.
#[derive(Debug, Clone)]
pub struct SessionClock {
    pub agent_id: String,
    pub clock: Arc<ActivityClock>,
}

tokio::task_local! { static CLOCK: SessionClock; }

/// The clock the current session reports to, if its host installed one.
pub fn current() -> Option<Arc<ActivityClock>> {
    CLOCK.try_with(|session| Arc::clone(&session.clock)).ok()
}

/// The installed clock, only when it was installed for `agent_id`.
pub fn current_for(agent_id: &str) -> Option<SessionClock> {
    CLOCK
        .try_with(Clone::clone)
        .ok()
        .filter(|session| session.agent_id == agent_id)
}

pub async fn scope<T>(session: SessionClock, work: impl std::future::Future<Output = T>) -> T {
    CLOCK.scope(session, work).await
}

/// Task locals do not cross spawn automatically. Capture on the caller with
/// [`current_for`] and explicitly restore around the spawned executor's work.
pub async fn inherit<T>(
    session: Option<SessionClock>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match session {
        Some(session) => scope(session, work).await,
        None => work.await,
    }
}

/// Record activity on the current session's clock; a no-op without one.
pub fn note() {
    if let Some(clock) = current() {
        clock.touch();
    }
}

/// Hold the current session active for a tool round; `None` without a clock.
pub fn tool_round() -> Option<ToolRoundGuard> {
    current().map(|clock| clock.tool_round())
}

#[cfg(test)]
#[path = "subagent_activity_tests.rs"]
mod tests;
