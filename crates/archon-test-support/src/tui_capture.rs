//! TUI frame capture helper for insta-based snapshot tests.
//!
//! Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-014.md
//! Based on:  02-technical-spec.md §1009 ("snapshot-test TUI frames
//!            with insta before-and-after each wave")
//! Derived from: REQ-FOR-D4 TUI modularization safety net
//!
//! ## What this module provides
//!
//! A tiny, dependency-free frame buffer + `AppLike` trait that
//! phase-4 modularization tasks will implement on the real `App`
//! from `archon-cli`. Paired with `insta::assert_snapshot!` this
//! gives the ~46-module main.rs refactor a deterministic
//! before-and-after diff per wave — any change in the rendered
//! frame is a regression.
//!
//! ## Why a local `FrameBuffer` instead of `ratatui::Buffer`
//!
//! `ratatui::Buffer` pulls the ratatui dep into `archon-test-support`
//! (a dev-only crate) which would leak into every crate that depends
//! on `archon-test-support`. Phase-0 is tests + scripts only — no
//! production dep graph touched. A 2D `Vec<Vec<char>>` is enough for
//! the snapshot contract (ASCII text, no colour, no attributes).
//!
//! ## Review workflow
//!
//! On drift, run `cargo insta review` and accept or reject per
//! snapshot. The `.snap` files live under
//! `tests/fixtures/snapshots/<crate>/` (see `insta.yaml` + the
//! `with_settings!` override in each test).

use std::fmt::Write as _;

/// Minimum trait that the real archon-cli `App` will implement in
/// phase-4. Exactly 3 methods by design — no scope creep.
pub trait AppLike {
    /// Feed one key event into the app state machine.
    fn handle_key(&mut self, key: char);
    /// Advance the app by one tick.
    fn tick(&mut self);
    /// Render the current app state into the supplied framebuffer.
    fn render_to_buffer(&self, buf: &mut FrameBuffer);
}

/// A width × height grid of ASCII characters. Space (`U+0020`) is the
/// empty cell. Lines are joined with `\n` on serialization with
/// trailing whitespace stripped per line so snapshots are stable
/// under cosmetic width changes.
#[derive(Debug, Clone)]
pub struct FrameBuffer {
    width: u16,
    height: u16,
    cells: Vec<Vec<char>>,
}

impl FrameBuffer {
    pub fn new(width: u16, height: u16) -> Self {
        let cells = (0..height).map(|_| vec![' '; width as usize]).collect();
        Self {
            width,
            height,
            cells,
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Write a single char at `(x, y)`. Out-of-bounds writes are
    /// silently dropped — matches ratatui semantics.
    pub fn set(&mut self, x: u16, y: u16, c: char) {
        if y >= self.height || x >= self.width {
            return;
        }
        self.cells[y as usize][x as usize] = c;
    }

    /// Write an ASCII string starting at `(x, y)`. Non-ASCII chars
    /// are replaced with `?` to keep snapshots byte-stable across
    /// locales. Truncates at row end.
    pub fn write_str(&mut self, x: u16, y: u16, s: &str) {
        for (i, ch) in s.chars().enumerate() {
            let c = if ch.is_ascii() && !ch.is_control() {
                ch
            } else {
                '?'
            };
            self.set(x + i as u16, y, c);
        }
    }

    /// Serialize to a normalized, insta-safe string: each row joined
    /// with `\n`, trailing whitespace stripped per row, final `\n`.
    pub fn to_normalized_string(&self) -> String {
        let mut out = String::with_capacity((self.width as usize + 1) * self.height as usize);
        for row in &self.cells {
            let line: String = row.iter().collect();
            let _ = write!(out, "{}\n", line.trim_end());
        }
        out
    }
}

/// Drive one tick + render cycle and return the normalized frame
/// string. This is the entry point every phase-4 snapshot test will
/// call.
pub fn render_frame_to_string<A: AppLike>(app: &mut A, width: u16, height: u16) -> String {
    let mut buf = FrameBuffer::new(width, height);
    app.tick();
    app.render_to_buffer(&mut buf);
    buf.to_normalized_string()
}

/// Trivial `AppLike` implementation that phase-0 tests (and the
/// initial baseline snapshot) use to exercise the helper without
/// depending on the real `App`. Renders `archon-cli vX.Y.Z` on
/// row 0.
#[derive(Debug, Default)]
pub struct DummyApp {
    pub tick_count: u32,
}

impl AppLike for DummyApp {
    fn handle_key(&mut self, _: char) {
        // no-op — DummyApp is stateless beyond tick_count
    }

    fn tick(&mut self) {
        self.tick_count = self.tick_count.saturating_add(1);
    }

    fn render_to_buffer(&self, buf: &mut FrameBuffer) {
        buf.write_str(0, 0, "archon-cli v0.1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_basic_write_and_normalize() {
        let mut b = FrameBuffer::new(10, 2);
        b.write_str(0, 0, "hi");
        let s = b.to_normalized_string();
        assert_eq!(s, "hi\n\n");
    }

    #[test]
    fn dummy_app_renders_banner() {
        let mut app = DummyApp::default();
        let s = render_frame_to_string(&mut app, 20, 3);
        assert!(s.contains("archon-cli"));
        assert_eq!(app.tick_count, 1);
    }
}
