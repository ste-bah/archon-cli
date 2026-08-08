//! The TUI must not panic on a short or narrow terminal.
//!
//! Found through the browser terminal pane (#150): a workbench pane that
//! fitted to 95x15 spawned the TUI, which panicked on its first frame with
//!
//! ```text
//! index outside of buffer: the area is Rect { x: 0, y: 0, width: 95, height: 15 }
//! but index is (0, 15)
//! ```
//!
//! Nothing about that is specific to a pseudo-terminal. A tiled window manager,
//! a split pane, or a browser window shorter than about 400px produces the same
//! size in a real terminal, and the same crash. The pane only made it easy to
//! hit, because a pane in a page is small by default in a way a terminal
//! application window is not.
//!
//! These draw at sizes an emulator can genuinely report and assert only that
//! the draw completes. Layout quality at 10 rows is not the subject — surviving
//! it is.

use archon_tui::app::App;
use archon_tui::render;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Draw one frame at `width` x `height`, returning nothing but propagating any
/// panic from inside the render pipeline.
fn draw_at(width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend");
    let mut app = App::new();
    terminal
        .draw(|frame| render::draw(frame, &mut app))
        .expect("draw");
}

#[test]
fn draws_at_the_size_that_crashed_the_web_terminal_pane() {
    draw_at(95, 15);
}

#[test]
fn draws_at_the_layout_minimum() {
    // 3 output + 5 input + 1 permission + 1 status is the floor the layout
    // constraints ask for. One row below it must still not panic.
    draw_at(80, 10);
    draw_at(80, 9);
}

#[test]
fn draws_at_sizes_a_split_pane_produces() {
    for (width, height) in [(40u16, 12u16), (60, 14), (95, 15), (120, 16), (200, 20)] {
        draw_at(width, height);
    }
}

#[test]
fn draws_when_the_terminal_is_narrow_as_well_as_short() {
    draw_at(20, 10);
    draw_at(12, 8);
}
