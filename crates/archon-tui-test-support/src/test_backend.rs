//! Ratatui `TestBackend` harness for TUI component tests.
//!
//! This module supplies a headless, deterministic harness used by phase-1
//! eventloop tests (REQ-TUI-LOOP-001 evidence) and phase-3 modularization
//! regression tests. It wraps a ratatui [`TestBackend`] in a small builder
//! API so callers can:
//!
//! * render frames into an in-memory buffer for golden snapshotting,
//! * simulate SIGWINCH resizes by resizing the underlying TestBackend,
//! * measure render latency under a paused tokio clock for deterministic
//!   timing assertions, and
//! * push synthetic `KeyEvent`s through a [`FakeInputStream`] without a TTY.
//!
//! # Usage for REQ-TUI-LOOP-001 evidence
//!
//! ```ignore
//! use archon_tui_test_support::test_backend::TuiHarness;
//! let mut h = TuiHarness::builder().size(80, 24).paused_clock().build();
//! h.render_frame(|f| {
//!     // draw widgets into the frame
//! });
//! let buf = h.buffer();
//! archon_tui_test_support::insta_wrapper::assert_buffer_snapshot("my_frame", buf);
//! ```

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use std::time::Duration;
use tokio::sync::mpsc;

// Re-export KeyEvent under a stable path so downstream tests
// don't need to depend on crossterm directly.
pub use ratatui::crossterm::event::KeyEvent;

/// Headless TUI harness wrapping a ratatui [`TestBackend`].
///
/// Built via [`TuiHarness::builder`]. Exposes a deterministic draw surface
/// plus helpers for simulating SIGWINCH resizes and measuring render latency
/// under a paused tokio clock.
pub struct TuiHarness {
    terminal: Terminal<TestBackend>,
    width: u16,
    height: u16,
    #[allow(dead_code)]
    paused_clock: bool,
}

impl TuiHarness {
    /// Begin constructing a harness.
    pub fn builder() -> TuiHarnessBuilder {
        TuiHarnessBuilder::default()
    }

    /// Render one frame using the supplied draw closure. The resulting
    /// buffer is retrievable via [`TuiHarness::buffer`].
    pub fn render_frame<F>(&mut self, draw: F)
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal
            .draw(|f| draw(f))
            .expect("TestBackend draw must not fail");
    }

    /// Borrow the current rendered buffer. Pair with
    /// `insta_wrapper::assert_buffer_snapshot` for phase-3 visual regression.
    pub fn buffer(&self) -> &Buffer {
        self.terminal.backend().buffer()
    }

    /// Simulate a SIGWINCH resize. The underlying `TestBackend` is resized
    /// in place and the harness dimensions are updated to match.
    pub fn resize(&mut self, w: u16, h: u16) {
        self.terminal.backend_mut().resize(w, h);
        self.width = w;
        self.height = h;
    }

    /// Render a frame and return the wall-clock duration spent rendering.
    ///
    /// When the caller has established a paused tokio clock (via
    /// `#[tokio::test(start_paused = true)]` or `tokio::time::pause()`)
    /// this duration is deterministic — it stays at zero unless the draw
    /// closure explicitly advances virtual time.
    pub fn measure_render_latency<F>(&mut self, draw: F) -> Duration
    where
        F: FnOnce(&mut Frame),
    {
        let start = tokio::time::Instant::now();
        self.render_frame(draw);
        let end = tokio::time::Instant::now();
        end.saturating_duration_since(start)
    }

    /// Reported harness dimensions.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }
}

/// Builder for [`TuiHarness`].
#[derive(Debug)]
pub struct TuiHarnessBuilder {
    width: u16,
    height: u16,
    paused_clock: bool,
}

impl Default for TuiHarnessBuilder {
    fn default() -> Self {
        Self {
            width: 80,
            height: 24,
            paused_clock: false,
        }
    }
}

impl TuiHarnessBuilder {
    /// Set the terminal dimensions. Defaults to 80x24.
    pub fn size(mut self, w: u16, h: u16) -> Self {
        self.width = w;
        self.height = h;
        self
    }

    /// Declare that callers will wrap the test in `#[tokio::test]` with
    /// `start_paused = true` (or otherwise call `tokio::time::pause()`).
    ///
    /// This is a marker flag — the actual clock pause must be established
    /// by the test via the tokio attribute macro or `tokio::time::pause()`.
    pub fn paused_clock(mut self) -> Self {
        self.paused_clock = true;
        self
    }

    /// Finalise and build the harness.
    pub fn build(self) -> TuiHarness {
        let backend = TestBackend::new(self.width, self.height);
        let terminal =
            Terminal::new(backend).expect("TestBackend terminal construction must not fail");
        TuiHarness {
            terminal,
            width: self.width,
            height: self.height,
            paused_clock: self.paused_clock,
        }
    }
}

/// A push-style wrapper around a tokio mpsc [`KeyEvent`] channel for tests
/// that need to simulate keyboard input without a real TTY.
pub struct FakeInputStream {
    tx: mpsc::UnboundedSender<KeyEvent>,
    rx: mpsc::UnboundedReceiver<KeyEvent>,
}

impl Default for FakeInputStream {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeInputStream {
    /// Create a new unbounded fake input stream.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    /// Push a key into the stream. Non-blocking.
    pub fn push(&self, key: KeyEvent) {
        let _ = self.tx.send(key);
    }

    /// Await the next key. Returns `None` if the stream is closed.
    pub async fn recv(&mut self) -> Option<KeyEvent> {
        self.rx.recv().await
    }

    /// Non-blocking `try_recv`. Returns `None` if the stream is empty.
    pub fn try_recv(&mut self) -> Option<KeyEvent> {
        self.rx.try_recv().ok()
    }
}
