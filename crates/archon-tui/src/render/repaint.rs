//! Full-repaint policy for geometry changes (#174 part 2, points 1 and 4).
//!
//! ratatui only writes the cells that differ between the frame it just built
//! and its model of the previous frame. That is correct exactly as long as its
//! model matches the screen. Every transition that *moves* content rather than
//! recolouring it — the input area growing or shrinking back, an overlay
//! opening or closing, a resize — is a chance for the two to part company, and
//! the cells left behind are the stray glyphs reported in #174.
//!
//! So the frame after any geometry change is preceded by a buffer-invalidating
//! [`Terminal::clear`], which resets ratatui's model to "nothing is on screen"
//! and forces the next frame to write every cell.
//!
//! **On resize this deliberately does nothing.** ratatui's `autoresize` already
//! calls `Terminal::clear` from inside `draw` whenever the reported size
//! differs from the last known one, so a clear here would be the second full
//! repaint of the same frame — one per step of a drag-resize, which is exactly
//! the flicker a blunt "clear on every resize event" produces. [`needs_clear`]
//! therefore suppresses its own clear when the area changed and leaves that
//! case to ratatui, and a resize event that reports an *unchanged* size (a
//! font change, a same-size SIGWINCH) still gets a clear through the forced
//! path because ratatui would skip it.
//!
//! [`needs_clear`]: RepaintTracker::needs_clear

use std::io;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;

use crate::app::App;

use super::body::input_display_text;
use super::layout::input_height_for_display;

/// Everything about a frame that decides *where* cells land.
///
/// Two frames with the same geometry can be diffed safely; two frames with
/// different geometry cannot, because content that moved leaves its old cells
/// untouched unless something repaints them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    area: Rect,
    input_height: u16,
    overlays: u32,
}

/// Fingerprint the geometry `app` would render into `area`.
pub fn frame_geometry(app: &App, area: Rect) -> FrameGeometry {
    FrameGeometry {
        area,
        input_height: input_height_for_display(area, &input_display_text(app)),
        overlays: overlay_mask(app),
    }
}

/// One bit per element that owns a floating or resizable footprint.
///
/// The bit positions carry no meaning beyond being stable within a single
/// process — only equality between consecutive frames is ever read.
fn overlay_mask(app: &App) -> u32 {
    let present = [
        app.show_splash,
        app.btw_overlay.is_some(),
        app.permission_prompt.is_some(),
        app.ask_user_prompt.is_some(),
        app.session_picker.is_some(),
        app.mcp_manager.is_some(),
        app.message_selector.is_some(),
        app.skills_menu.is_some(),
        app.file_picker.is_some(),
        app.search_results.is_some(),
        app.evidence_view.is_some(),
        app.thinking_archive.is_some(),
        app.vim_state.is_some(),
        app.input.suggestions.active && !app.is_generating,
        app.activity_stream.is_foreground(),
        !app.agent_activity.is_empty(),
        app.output.scroll_locked,
    ];
    present
        .iter()
        .enumerate()
        .fold(0u32, |mask, (bit, &on)| mask | (u32::from(on) << bit))
}

/// Remembers the last drawn geometry and decides when a frame needs a full
/// repaint rather than a diff.
#[derive(Debug, Default)]
pub struct RepaintTracker {
    last: Option<FrameGeometry>,
    forced: bool,
}

impl RepaintTracker {
    /// Request that the next frame repaint every cell.
    ///
    /// The manual escape hatch (Ctrl+L) and same-size resize notifications go
    /// through here.
    pub fn request_full_repaint(&mut self) {
        self.forced = true;
    }

    /// Whether `current` must be preceded by `Terminal::clear()`.
    ///
    /// Consumes the forced flag and records `current` as the last geometry, so
    /// each call corresponds to exactly one frame.
    fn needs_clear(&mut self, current: FrameGeometry) -> bool {
        let forced = std::mem::replace(&mut self.forced, false);
        let Some(previous) = self.last.replace(current) else {
            // First frame of the session: ratatui's model already says the
            // screen is empty, so the frame writes every cell regardless.
            return false;
        };
        if previous.area != current.area {
            // ratatui::Terminal::autoresize clears for us inside `draw`.
            return false;
        }
        forced || previous != current
    }
}

/// Draw one frame, preceded by a full clear when the geometry moved.
///
/// This is the only draw path the live loop uses, and the one the artifact
/// harness drives, so the clear policy is exercised by both.
///
/// The size is read before the draw rather than reused from the last frame:
/// the decision turns on whether the size *changed*, so a stale value would
/// answer the wrong question. That costs one extra size query per frame on top
/// of the one `Terminal::draw` already makes — an ioctl at the loop's idle
/// cadence of four frames a second.
pub fn draw_frame<B>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    tracker: &mut RepaintTracker,
) -> io::Result<()>
where
    B: Backend,
{
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    if tracker.needs_clear(frame_geometry(app, area)) {
        terminal.clear()?;
    }
    terminal.draw(|frame| super::draw(frame, app))?;
    Ok(())
}

/// Give `tracker` a look at a terminal event before the input dispatch does.
///
/// Returns `true` when the event was consumed here and must not be dispatched
/// further — currently only Ctrl+L, the conventional "redraw the screen"
/// binding, which is deliberately handled outside the keymap so it works from
/// inside every overlay and modal.
pub fn note_terminal_event(event: &Event, tracker: &mut RepaintTracker) -> bool {
    match event {
        Event::Key(key)
            if key.kind != KeyEventKind::Release
                && key.code == KeyCode::Char('l')
                && key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            tracker.request_full_repaint();
            true
        }
        Event::Resize(..) => {
            tracker.request_full_repaint();
            false
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "repaint_tests.rs"]
mod tests;
