//! TUI render pipeline.
//!
//! ## Structure
//!
//! - `layout` — `Layout` struct and `compute_layout()` for region computation
//! - `chrome` — status bar, permission indicator, /btw overlay
//! - `body` — output area, input area, session badge, suggestions popup,
//!   MCP manager overlay, session picker overlay
//! - `mod.rs` — facade: `draw()` function entry point
//!
//! ## Usage
//!
//! ```ignore
//! use crate::render::draw;
//! terminal.draw(|frame| {
//!     draw(frame, app);
//! })?;
//! ```

pub mod body;
pub mod chrome;
pub mod cursor;
pub mod layout;

pub use layout::{Layout, compute_layout};

use crate::app::App;
use ratatui::Frame;

/// Main draw entry point — renders the entire TUI frame.
///
/// Call this from `terminal.draw()` instead of an inline closure:
/// ```ignore
/// terminal.draw(|frame| {
///     draw(frame, app);
/// })?;
/// ```
pub fn draw(frame: &mut Frame, app: &mut App) {
    let size = frame.area();
    let l = compute_layout(size);

    // Body: output area
    body::draw_output_area(frame, app, l.output);

    // Body: input area
    body::draw_input_area(frame, app, l.input);

    // Body: session badge
    body::draw_session_badge(frame, app, l.input);

    // Body: suggestions popup
    body::draw_suggestions_popup(frame, app, l.input);

    // Chrome: permission indicator
    chrome::draw_permission_indicator(frame, app, l.permission);

    // Chrome: status bar
    chrome::draw_status_bar(frame, app, l.status);

    // Chrome: /btw overlay
    chrome::draw_btw_overlay(frame, app);

    // Overlays: session picker
    body::draw_session_picker(frame, app);

    // Overlays: MCP manager
    body::draw_mcp_manager(frame, app);

    // Overlays: message selector (TASK-TUI-620 /rewind)
    body::draw_message_selector(frame, app);

    // Overlays: skills menu (TASK-TUI-627 /skills)
    body::draw_skills_menu(frame, app);

    // Overlays: file picker (TASK-#207 /files)
    body::draw_file_picker(frame, app);

    // Overlays: search results (TASK-#208 /search)
    body::draw_search_results(frame, app);
}
