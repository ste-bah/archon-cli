//! Live terminals, and the rules that stop them accumulating (#189 Phase 6).
//!
//! A persistent shell is a process that outlives the tool call that made it, so
//! the interesting part is not creating one — it is being certain every one of
//! them ends. Three rules do that, and each covers a case the others miss:
//! a cap, so a loop cannot spawn shells without limit; an idle timeout, so a
//! forgotten terminal does not live as long as the session; and a close on
//! session end, which is the only one that catches a terminal still in use when
//! the user quits.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use archon_pty::{CommandBuilder, PtyControl, PtySession, PtySize};
use dashmap::DashMap;
use once_cell::sync::Lazy;

use crate::terminal_buffer::{BufferRead, OutputBuffer};

/// How many terminals one process will hold open at once.
///
/// Each is a live shell with two pump threads, so this is a real resource, and
/// an agent that needs more than a handful at once has almost certainly lost
/// track of the ones it has.
pub(crate) const MAX_TERMINALS: usize = 8;

/// How long a terminal may go untouched before it is closed.
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Rows and columns reported to the shell.
///
/// Nothing renders this output, but the size still matters: at the default
/// 80 columns a shell wraps long lines with escape sequences, and the wrapping
/// survives into what the model reads. A wide terminal is a cheap way to make
/// the text come back as it was written.
const TERMINAL_SIZE: PtySize = PtySize {
    rows: 50,
    cols: 240,
    pixel_width: 0,
    pixel_height: 0,
};

static TERMINALS: Lazy<DashMap<String, Arc<Terminal>>> = Lazy::new(DashMap::new);

/// One live shell, its accumulated output, and who it belongs to.
pub(crate) struct Terminal {
    pub(crate) id: String,
    /// The agent session that opened it, so session end can close its own.
    session_id: String,
    pub(crate) shell: String,
    control: Arc<PtyControl>,
    buffer: Mutex<OutputBuffer>,
    last_used: Mutex<Instant>,
}

impl Terminal {
    pub(crate) fn write(&self, text: &str) {
        self.touch();
        self.control.send_input(text.as_bytes().to_vec());
    }

    pub(crate) fn read(&self, since: u64, max_bytes: usize) -> BufferRead {
        self.touch();
        self.buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .read_from(since, max_bytes)
    }

    /// Total bytes this terminal has produced, for a caller that wants to know
    /// where it stands without reading.
    pub(crate) fn produced(&self) -> u64 {
        self.buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .end()
    }

    fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(
            *self
                .last_used
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    fn push(&self, chunk: &[u8]) {
        self.buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(chunk);
    }
}

/// Open a shell and register it.
///
/// Reaping happens here rather than on a timer: every route to a new terminal
/// passes through this function, so the cap is never enforced against corpses,
/// and no background task has to be shut down in turn. A terminal that goes
/// idle while nothing else is happening is closed by [`close_session`] instead.
pub(crate) fn create(
    session_id: &str,
    id: String,
    shell: String,
    program: CommandBuilder,
) -> Result<Arc<Terminal>, String> {
    reap_idle();
    if TERMINALS.len() >= MAX_TERMINALS {
        return Err(format!(
            "{MAX_TERMINALS} terminals are already open, which is the limit. \
             Close one with TerminalClose before opening another."
        ));
    }

    let session = PtySession::spawn_headless(program, TERMINAL_SIZE)
        .map_err(|error| format!("could not open a terminal: {error}"))?;
    let (control, mut output) = session.split();

    let terminal = Arc::new(Terminal {
        id: id.clone(),
        session_id: session_id.to_string(),
        shell,
        control,
        buffer: Mutex::new(OutputBuffer::new()),
        last_used: Mutex::new(Instant::now()),
    });

    // The drain task holds a `Weak`, so it can never be the reason a terminal
    // stays alive. Dropping the registry entry drops the last strong reference,
    // which kills the child, which ends this stream — so the task retires on
    // its own without anything having to signal it.
    let weak: Weak<Terminal> = Arc::downgrade(&terminal);
    tokio::spawn(async move {
        while let Some(chunk) = output.recv().await {
            let Some(terminal) = weak.upgrade() else {
                break;
            };
            terminal.push(&chunk);
        }
    });

    // Logged because a persistent shell is the one tool effect that is still
    // there after the call that caused it: an operator looking at a stray
    // process needs a line saying which session opened it and what it is.
    tracing::info!(
        terminal = %terminal.id,
        shell = %terminal.shell,
        session = %terminal.session_id,
        "terminal: opened a persistent shell"
    );
    TERMINALS.insert(id, Arc::clone(&terminal));
    Ok(terminal)
}

pub(crate) fn get(id: &str) -> Option<Arc<Terminal>> {
    TERMINALS.get(id).map(|entry| Arc::clone(entry.value()))
}

/// Close one terminal. `false` means there was no such terminal.
pub(crate) fn close(id: &str) -> bool {
    let Some((_, terminal)) = TERMINALS.remove(id) else {
        return false;
    };
    // Killed here rather than left to the `Arc` going out of scope: a caller
    // may still be holding a handle from `get`, and "closed" has to mean the
    // process is gone at that moment, not whenever the last reader lets go.
    terminal.control.kill();
    true
}

/// Ids of every open terminal belonging to `session_id`.
pub(crate) fn ids_for_session(session_id: &str) -> Vec<String> {
    let mut ids: Vec<String> = TERMINALS
        .iter()
        .filter(|entry| entry.value().session_id == session_id)
        .map(|entry| entry.key().clone())
        .collect();
    ids.sort();
    ids
}

/// Close every terminal opened by one session. Returns how many were closed.
///
/// This is the rule that has to hold on the way out: the cap and the idle
/// timeout both leave a terminal running if it is under the limit and recently
/// used, which is exactly the state a terminal is in when a session ends.
pub(crate) fn close_session(session_id: &str) -> usize {
    let ids = ids_for_session(session_id);
    ids.iter().filter(|id| close(id)).count()
}

pub(crate) fn reap_idle() -> usize {
    reap_idle_at(Instant::now(), IDLE_TIMEOUT)
}

/// Split from [`reap_idle`] so the rule can be tested without waiting half an
/// hour for it.
pub(crate) fn reap_idle_at(now: Instant, timeout: Duration) -> usize {
    let stale: Vec<String> = TERMINALS
        .iter()
        .filter(|entry| entry.value().idle_for(now) >= timeout)
        .map(|entry| entry.key().clone())
        .collect();
    for id in &stale {
        tracing::info!(terminal = %id, "terminal: closing an idle session");
    }
    stale.iter().filter(|id| close(id)).count()
}

#[cfg(test)]
#[path = "terminal_registry_tests.rs"]
mod tests;
