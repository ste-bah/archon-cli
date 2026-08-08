//! PTY plumbing for the browser terminal pane.
//!
//! Split out from `terminal.rs` so that file stays about the security gate and
//! the WebSocket protocol, and this one is only about moving bytes.
//!
//! `portable-pty` is blocking on both ends — ConPTY and `openpty` both hand
//! back plain `Read`/`Write` handles with no async story — so each direction
//! gets a dedicated OS thread bridged to the async handler by an mpsc channel.
//! `spawn_blocking` would work too, but these threads live for the whole
//! connection and would occupy the blocking pool for the same duration, which
//! is exactly what that pool is documented not to be for.

use std::io::{Read, Write};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;

/// Read buffer for one PTY read. A full-screen ratatui repaint at a large
/// terminal size is a few kilobytes of escape sequences, so this usually moves
/// a whole frame per wakeup instead of shredding it across messages.
const READ_CHUNK: usize = 8 * 1024;

/// Queued output chunks. Bounded on purpose: a client that has stopped reading
/// must eventually apply backpressure to the reader thread rather than let the
/// process grow without limit. At `READ_CHUNK` each that is a ~2 MiB ceiling.
const OUTPUT_QUEUE: usize = 256;

/// Queued input chunks. Keystrokes are bytes, but a paste arrives as one large
/// chunk, so this is sized for bursts rather than for typing speed.
const INPUT_QUEUE: usize = 1024;

/// One PTY and the child running inside it, for the lifetime of one WebSocket.
///
/// Not `Clone` and not shared: a terminal has exactly one owner, and the `Drop`
/// impl below is what guarantees the child cannot outlive its socket.
pub(super) struct PtySession {
    /// Kept alive for `resize`, and because dropping it is what makes the
    /// reader thread see EOF.
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    output: mpsc::Receiver<Vec<u8>>,
    input: mpsc::Sender<Vec<u8>>,
    /// For log lines only; the handle that can actually stop it is `killer`.
    child_pid: Option<u32>,
}

impl PtySession {
    /// Open a PTY, spawn `command` inside it, and start both pumps.
    pub(super) fn spawn(command: CommandBuilder, size: PtySize) -> anyhow::Result<Self> {
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| anyhow::anyhow!("terminal: openpty failed: {error}"))?;

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| anyhow::anyhow!("terminal: spawn failed: {error}"))?;
        let child_pid = child.process_id();
        let killer = child.clone_killer();

        // Drop our copy of the slave immediately. While this process still
        // holds it the PTY has a writer that will never write, so the reader
        // below would block forever instead of seeing EOF when the child exits
        // — the connection would then hang open around a dead process.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| anyhow::anyhow!("terminal: pty reader failed: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| anyhow::anyhow!("terminal: pty writer failed: {error}"))?;

        let (out_tx, output) = mpsc::channel(OUTPUT_QUEUE);
        let (input, in_rx) = mpsc::channel(INPUT_QUEUE);

        spawn_named("archon-web-pty-read", move || pump_output(reader, out_tx));
        spawn_named("archon-web-pty-write", move || pump_input(writer, in_rx));
        // Reaping happens on its own thread because `wait` blocks. Without it
        // the child would linger as a zombie on unix after `kill`, and nothing
        // else in this design is positioned to call `wait`: the handler task
        // may be gone by the time the child actually dies.
        spawn_named("archon-web-pty-wait", move || {
            let _ = child.wait();
        });

        Ok(Self {
            master: pair.master,
            killer,
            output,
            input,
            child_pid,
        })
    }

    /// Next chunk of PTY output, or `None` once the child's output has ended.
    pub(super) async fn next_output(&mut self) -> Option<Vec<u8>> {
        self.output.recv().await
    }

    /// Queue bytes for the child's stdin.
    ///
    /// Deliberately non-blocking. Awaiting a full queue would stop this task
    /// draining `output` too, and a child that has stopped reading its input
    /// while still writing would then stall the whole connection rather than
    /// just losing the keystrokes it was never going to consume.
    pub(super) fn send_input(&self, bytes: Vec<u8>) {
        if self.input.try_send(bytes).is_err() {
            tracing::warn!("terminal: input queue full or closed; dropped keystrokes");
        }
    }

    /// Propagate the browser's terminal size to the PTY.
    ///
    /// This is not cosmetic. A full-screen ratatui app draws to the size the
    /// kernel reports; render it at the wrong width and the output is visibly
    /// corrupted rather than reflowed, because the escape sequences address
    /// columns that are not where the emulator thinks they are.
    pub(super) fn resize(&self, rows: u16, cols: u16) {
        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        if let Err(error) = self.master.resize(size) {
            tracing::warn!(%error, "terminal: pty resize failed");
        }
    }

    pub(super) fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }
}

impl Drop for PtySession {
    /// The single place the "a terminal must not outlive its tab" rule is
    /// enforced. Putting it in `Drop` rather than on the socket-closed path
    /// means it also holds when the handler task is cancelled or unwinds,
    /// which are exactly the paths a leaked shell would escape through.
    fn drop(&mut self) {
        if let Err(error) = self.killer.kill() {
            // Already-exited is the common case here, not a failure.
            tracing::debug!(%error, "terminal: kill on drop returned an error");
        }
        tracing::info!(pid = ?self.child_pid, "terminal: session closed, child killed");
    }
}

fn spawn_named(name: &str, body: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
    {
        tracing::error!(%error, name, "terminal: could not spawn pty pump thread");
    }
}

fn pump_output(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        }
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
