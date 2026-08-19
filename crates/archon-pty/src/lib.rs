//! A pseudo-terminal and the child running inside it.
//!
//! Moved down out of `archon-sdk`'s browser terminal pane so the persistent
//! shell tools in `archon-tools` run on the same plumbing (#189 Phase 6).
//! `archon-tools` cannot depend on `archon-sdk` — that is the wrong direction
//! through the layer graph — so sharing had to mean moving the code down
//! rather than reaching sideways for it. Writing a second implementation was
//! the cheaper alternative and was rejected: two PTY lifecycles is two places
//! for a shell to be leaked, and only one of them would have the `Drop` impl
//! below.
//!
//! Deliberately a leaf — no `archon-*` dependencies — so anything above it can
//! use it. `archon-shell` was the obvious existing home and was passed over: it
//! is a `which` lookup that much of the workspace depends on, and hanging tokio
//! and ConPTY off it would make every one of those dependents pay for a PTY
//! they never open.
//!
//! `portable-pty` is blocking on both ends — ConPTY and `openpty` both hand
//! back plain `Read`/`Write` handles with no async story — so each direction
//! gets a dedicated OS thread bridged to async callers by an mpsc channel.
//! `spawn_blocking` would work too, but these threads live for as long as the
//! session does and would occupy the blocking pool for that whole time, which
//! is exactly what that pool is documented not to be for.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{ChildKiller, MasterPty, native_pty_system};
pub use portable_pty::{CommandBuilder, PtySize};
use tokio::sync::mpsc;

/// Read buffer for one PTY read. A full-screen ratatui repaint at a large
/// terminal size is a few kilobytes of escape sequences, so this usually moves
/// a whole frame per wakeup instead of shredding it across messages.
const READ_CHUNK: usize = 8 * 1024;

/// Queued output chunks. Bounded on purpose: a consumer that has stopped
/// reading must eventually apply backpressure to the reader thread rather than
/// let the process grow without limit. At `READ_CHUNK` each that is a ~2 MiB
/// ceiling.
const OUTPUT_QUEUE: usize = 256;

/// Queued input chunks. Keystrokes are bytes, but a paste — or a whole command
/// line from a tool call — arrives as one large chunk, so this is sized for
/// bursts rather than for typing speed.
const INPUT_QUEUE: usize = 1024;

/// Cursor-position report request (DSR 6), which ConPTY emits on startup.
///
/// It then *waits* for the answer before letting the shell produce anything.
/// With no reply, even `cmd.exe /C echo hi` yields exactly these four bytes and
/// nothing else — verified, and the reason [`PtySession::spawn_headless`]
/// exists. The browser pane never hit this because xterm.js answers it.
const DEVICE_STATUS_REQUEST: &[u8] = b"\x1b[6n";

/// "The cursor is at row 1, column 1" — what a fresh terminal would report.
///
/// ConPTY only needs *an* answer to proceed; it re-queries whenever it needs
/// the real position, so a constant reply keeps it unblocked without pretending
/// to track a screen this side does not render.
const DEVICE_STATUS_REPLY: &[u8] = b"\x1b[1;1R";

/// Everything needed to drive a live PTY, minus its output stream.
///
/// Split from the output receiver so the two can be owned separately: a
/// consumer that drains output on a background task still needs to write,
/// resize and kill from elsewhere, and a receiver cannot be shared. `Clone` is
/// deliberately absent — share it as `Arc<PtyControl>` so there is exactly one
/// `Drop`, which is the whole safety story.
pub struct PtyControl {
    /// Kept alive for `resize`, and because dropping it is what makes the
    /// reader thread see EOF. `Mutex` because `MasterPty` is `Send` but not
    /// `Sync`, and this handle is shared.
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    input: mpsc::Sender<Vec<u8>>,
    /// For log lines only; the handle that can actually stop it is `killer`.
    child_pid: Option<u32>,
}

impl PtyControl {
    /// Queue bytes for the child's stdin.
    ///
    /// Deliberately non-blocking. Awaiting a full queue would stop the caller
    /// draining output too, and a child that has stopped reading its input
    /// while still writing would then stall everything rather than just losing
    /// the bytes it was never going to consume.
    pub fn send_input(&self, bytes: Vec<u8>) {
        if self.input.try_send(bytes).is_err() {
            tracing::warn!(pid = ?self.child_pid, "pty: input queue full or closed; dropped input");
        }
    }

    /// Propagate a terminal size to the PTY.
    ///
    /// This is not cosmetic. A full-screen application draws to the size the
    /// kernel reports; render it at the wrong width and the output is visibly
    /// corrupted rather than reflowed, because the escape sequences address
    /// columns that are not where the emulator thinks they are.
    pub fn resize(&self, rows: u16, cols: u16) {
        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let resized = self
            .master
            .lock()
            .map_err(|_| ())
            .and_then(|master| master.resize(size).map_err(|_| ()));
        if resized.is_err() {
            tracing::warn!(pid = ?self.child_pid, "pty: resize failed");
        }
    }

    /// Stop the child now.
    ///
    /// Idempotent: killing an already-dead child is the common case, not a
    /// failure, so this reports nothing. Exposed as well as being called from
    /// `Drop` so an owner holding a shared handle can end the child at a chosen
    /// moment instead of waiting for the last reference to go.
    ///
    /// This does **not** end the output stream on Windows: the reader sees EOF
    /// when the ConPTY itself closes, which happens when this handle drops, not
    /// when the child dies. So "killed" and "output finished" are two events,
    /// and an owner that needs the second must drop the handle.
    pub fn kill(&self) {
        if let Ok(mut killer) = self.killer.lock()
            && let Err(error) = killer.kill()
        {
            tracing::debug!(%error, pid = ?self.child_pid, "pty: kill returned an error");
        }
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }
}

impl Drop for PtyControl {
    /// The single place the "a child must not outlive its owner" rule is
    /// enforced. Putting it in `Drop` rather than on each close path means it
    /// also holds when a task is cancelled or unwinds, which are exactly the
    /// paths a leaked shell would escape through.
    fn drop(&mut self) {
        self.kill();
        tracing::info!(pid = ?self.child_pid, "pty: session closed, child killed");
    }
}

/// One PTY and the child running inside it.
///
/// Not `Clone`: a terminal has exactly one owner. Callers that want output on
/// one task and control on another take [`PtySession::split`].
pub struct PtySession {
    control: Arc<PtyControl>,
    output: mpsc::Receiver<Vec<u8>>,
}

impl PtySession {
    /// Open a PTY, spawn `command` inside it, and start both pumps.
    ///
    /// For consumers with a real terminal emulator on the far end — the browser
    /// pane, where xterm.js answers the terminal queries ConPTY asks. Anything
    /// else wants [`Self::spawn_headless`].
    pub fn spawn(command: CommandBuilder, size: PtySize) -> anyhow::Result<Self> {
        Self::spawn_with(command, size, false)
    }

    /// As [`Self::spawn`], but answer the terminal queries nobody else will.
    ///
    /// Without this a headless caller gets four bytes and then silence: ConPTY
    /// opens by asking for the cursor position and blocks the shell until it is
    /// told. A browser has an emulator to answer; a tool call does not.
    pub fn spawn_headless(command: CommandBuilder, size: PtySize) -> anyhow::Result<Self> {
        Self::spawn_with(command, size, true)
    }

    fn spawn_with(
        command: CommandBuilder,
        size: PtySize,
        answer_device_reports: bool,
    ) -> anyhow::Result<Self> {
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| anyhow::anyhow!("pty: openpty failed: {error}"))?;

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| anyhow::anyhow!("pty: spawn failed: {error}"))?;
        let child_pid = child.process_id();
        let killer = child.clone_killer();

        // Drop our copy of the slave immediately. While this process still
        // holds it the PTY has a writer that will never write, so the reader
        // below would block forever instead of seeing EOF when the child exits
        // — the session would then hang open around a dead process.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| anyhow::anyhow!("pty: reader failed: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| anyhow::anyhow!("pty: writer failed: {error}"))?;

        let (out_tx, output) = mpsc::channel(OUTPUT_QUEUE);
        let (input, in_rx) = mpsc::channel(INPUT_QUEUE);

        let answer = answer_device_reports.then(|| input.clone());
        spawn_named("archon-pty-read", move || {
            pump_output(reader, out_tx, answer)
        });
        spawn_named("archon-pty-write", move || pump_input(writer, in_rx));
        // Reaping happens on its own thread because `wait` blocks. Without it
        // the child would linger as a zombie on unix after `kill`, and nothing
        // else in this design is positioned to call `wait`: the owning task may
        // be gone by the time the child actually dies.
        spawn_named("archon-pty-wait", move || {
            let _ = child.wait();
        });

        Ok(Self {
            control: Arc::new(PtyControl {
                master: Mutex::new(pair.master),
                killer: Mutex::new(killer),
                input,
                child_pid,
            }),
            output,
        })
    }

    /// Next chunk of PTY output, or `None` once the child's output has ended.
    pub async fn next_output(&mut self) -> Option<Vec<u8>> {
        self.output.recv().await
    }

    /// Hand out the control half and the output stream separately.
    ///
    /// The returned `Arc` is the only one in existence, so dropping it still
    /// kills the child. The receiver ends on its own once the reader thread
    /// sees EOF, which is what killing the child causes — so an owner needs to
    /// track only the control half to shut the whole thing down.
    pub fn split(self) -> (Arc<PtyControl>, mpsc::Receiver<Vec<u8>>) {
        (self.control, self.output)
    }

    pub fn send_input(&self, bytes: Vec<u8>) {
        self.control.send_input(bytes);
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.control.resize(rows, cols);
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.control.child_pid()
    }
}

fn spawn_named(name: &str, body: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
    {
        tracing::error!(%error, name, "pty: could not spawn pump thread");
    }
}

fn pump_output(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<Vec<u8>>,
    answer: Option<mpsc::Sender<Vec<u8>>>,
) {
    let mut buf = vec![0u8; READ_CHUNK];
    // Last few bytes of the previous chunk, so a request straddling a read
    // boundary is still recognised. Nothing is re-emitted from it — it only
    // widens the window the scan looks through.
    let mut carry: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                if let Some(answer) = &answer {
                    answer_device_status(chunk, &mut carry, answer);
                }
                if tx.blocking_send(chunk.to_vec()).is_err() {
                    break;
                }
            }
        }
    }
}

/// Reply once per cursor-position request seen in `chunk`.
///
/// Per request rather than per chunk: ConPTY asks again after a resize, and a
/// missed second question stalls the shell exactly as the first one would.
fn answer_device_status(chunk: &[u8], carry: &mut Vec<u8>, answer: &mpsc::Sender<Vec<u8>>) {
    carry.extend_from_slice(chunk);
    let requests = carry
        .windows(DEVICE_STATUS_REQUEST.len())
        .filter(|window| *window == DEVICE_STATUS_REQUEST)
        .count();
    for _ in 0..requests {
        if answer.try_send(DEVICE_STATUS_REPLY.to_vec()).is_err() {
            tracing::warn!("pty: could not answer a cursor-position request; the shell may stall");
            break;
        }
    }
    // Keep only enough tail to complete a request split across the boundary,
    // and drop the rest so this does not grow into a copy of the session. Three
    // bytes cannot themselves contain a four-byte request, which is also what
    // stops a match at the end of one chunk being counted again in the next.
    let keep = DEVICE_STATUS_REQUEST.len() - 1;
    if carry.len() > keep {
        carry.drain(..carry.len() - keep);
    }
}

fn pump_input(mut writer: Box<dyn Write + Send>, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(bytes) = rx.blocking_recv() {
        if writer.write_all(&bytes).is_err() {
            break;
        }
        // Flushed per message: a keystroke that sits in a buffer is a terminal
        // that appears not to respond.
        if writer.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
