//! #174 part 2 acceptance harness: no cell of the screen may survive a
//! geometry change unaccounted for.
//!
//! The artifact reported in #174 — digits at spaced columns and a `%` at the
//! right margin, appearing in and around the input area after a submit — is
//! screen state that nothing repainted. That is only observable against a
//! model of the *physical* screen, so this harness supplies one
//! ([`ScreenModel`]) and drives the production draw path against it through
//! 100 scripted cycles of {type wrapped input → submit → overlay open/close →
//! resize ±1 col}. After every single frame the modelled screen is compared,
//! cell by cell, with a frame rendered from scratch into an empty screen of
//! the same size. Zero cells may differ.
//!
//! The distinction this buys: a spot check tells you the artifact stopped
//! reproducing; this tells you the screen cannot hold a cell the current
//! frame did not put there.

use std::io;

use archon_tui::app::{App, SessionPicker, SessionPickerEntry};
use archon_tui::render::{RepaintTracker, draw_frame};
use ratatui::Terminal;
use ratatui::backend::{Backend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use unicode_width::UnicodeWidthStr;

// ───────────────────────────────────────────────────────────────────────────
// A model of the physical screen
// ───────────────────────────────────────────────────────────────────────────

/// A backend that models a real terminal rather than ratatui's idea of one.
///
/// `TestBackend` files every written cell at the coordinate ratatui names, so
/// it can never disagree with ratatui about where a glyph landed. A terminal
/// has no such luxury: it advances its cursor by the *display width* of what
/// was printed, and `CrosstermBackend` only emits a cursor move when the next
/// cell is not adjacent to the last one. A glyph whose display width differs
/// from the width ratatui reserved therefore shifts everything printed after
/// it in the same run — which is how a status line like `ctx 392k/1000k (39%)`
/// ends up scattering `1`, `0` and `%` into cells nobody owns.
///
/// This backend reproduces both rules, so a width bug anywhere in the draw
/// path shows up here as cells that differ from a clean render.
struct ScreenModel {
    width: u16,
    height: u16,
    /// Row-major grid. A cell hidden underneath a double-width glyph holds an
    /// empty string, matching how ratatui reserves the trailing column.
    cells: Vec<String>,
    cursor: Position,
    cursor_visible: bool,
}

impl ScreenModel {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cells: vec![" ".to_string(); width as usize * height as usize],
            cursor: Position::ORIGIN,
            cursor_visible: true,
        }
    }

    fn blank(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
            cell.push(' ');
        }
    }

    /// Resize the screen the way a terminal does: the overlapping region keeps
    /// whatever was already on it. Blanking here instead would hide exactly
    /// the leftovers this harness exists to find.
    fn resize(&mut self, width: u16, height: u16) {
        let mut cells = vec![" ".to_string(); width as usize * height as usize];
        for y in 0..height.min(self.height) {
            for x in 0..width.min(self.width) {
                cells[y as usize * width as usize + x as usize] =
                    self.cells[y as usize * self.width as usize + x as usize].clone();
            }
        }
        self.width = width;
        self.height = height;
        self.cells = cells;
        self.cursor = Position::ORIGIN;
    }

    /// Print one grapheme at the cursor and advance by its display width, with
    /// the auto-wrap every terminal has on by default.
    ///
    /// Content pushed past the last row is dropped rather than scrolled: this
    /// harness is about cells that stay, and modelling a scroll would move
    /// every row instead.
    fn print(&mut self, symbol: &str) {
        let advance = UnicodeWidthStr::width(symbol).max(1) as u16;
        if self.cursor.x >= self.width {
            self.cursor = Position::new(0, self.cursor.y.saturating_add(1));
        }
        if self.cursor.y < self.height {
            for column in 0..advance {
                let x = self.cursor.x.saturating_add(column);
                if x >= self.width {
                    break;
                }
                let index = self.cursor.y as usize * self.width as usize + x as usize;
                self.cells[index] = if column == 0 {
                    symbol.to_string()
                } else {
                    String::new()
                };
            }
        }
        self.cursor.x = self.cursor.x.saturating_add(advance);
    }
}

impl Backend for ScreenModel {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut previous: Option<(u16, u16)> = None;
        for (x, y, cell) in content {
            // CrosstermBackend's rule: skip the cursor move when this cell is
            // the immediate successor of the last one. That is what lets a
            // mis-measured glyph drag its neighbours out of place.
            if !matches!(previous, Some((px, py)) if y == py && x == px + 1) {
                self.cursor = Position::new(x, y);
            }
            self.print(cell.symbol());
            previous = Some((x, y));
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.cursor_visible = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.cursor_visible = true;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.blank();
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.width, self.height))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size::new(self.width, self.height),
            pixels: Size::new(self.width * 8, self.height * 16),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Harness
// ───────────────────────────────────────────────────────────────────────────

/// Render `app` into a screen that starts empty, and hand back its cells.
///
/// This is the reference: what the terminal would show if it had been painted
/// once, from nothing, in exactly this state.
fn freshly_rendered(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(ScreenModel::new(width, height)).expect("fresh terminal");
    let mut repaint = RepaintTracker::default();
    draw_frame(&mut terminal, app, &mut repaint).expect("fresh draw");
    terminal.backend().cells.clone()
}

/// Compare the live screen against a clean render of the same state and
/// return every differing cell as `(x, y, live, fresh)`.
fn stray_cells(live: &[String], fresh: &[String], width: u16) -> Vec<(u16, u16, String, String)> {
    assert_eq!(
        live.len(),
        fresh.len(),
        "the two screens must cover the same cells, or zip would hide the tail"
    );
    live.iter()
        .zip(fresh.iter())
        .enumerate()
        .filter(|(_, (live, fresh))| live != fresh)
        .map(|(index, (live, fresh))| {
            let x = (index % width as usize) as u16;
            let y = (index / width as usize) as u16;
            (x, y, live.clone(), fresh.clone())
        })
        .collect()
}

/// Draw one live frame, then assert the screen holds nothing a clean render
/// would not have put there.
fn draw_and_assert_clean(
    terminal: &mut Terminal<ScreenModel>,
    app: &mut App,
    repaint: &mut RepaintTracker,
    step: &str,
) {
    draw_frame(terminal, app, repaint).expect("live draw");
    let (width, height) = (terminal.backend().width, terminal.backend().height);
    let live = terminal.backend().cells.clone();
    let fresh = freshly_rendered(app, width, height);

    let strays = stray_cells(&live, &fresh, width);
    assert!(
        strays.is_empty(),
        "{step}: {} cell(s) on a {width}x{height} screen differ from a freshly \
         rendered frame — leftovers nothing repainted.\nfirst 12: {:?}",
        strays.len(),
        strays.iter().take(12).collect::<Vec<_>>()
    );
}

/// A status line carrying the shape from the bug report plus double-width
/// text, so the status bar is measured in columns on every frame.
fn app_with_wide_status() -> App {
    let mut app = App::new();
    app.show_splash = false;
    app.status.model = "claude-opus-4-7".into();
    app.status.context_window = 1_000_000;
    app.status.context_tokens_used = 392_000;
    app.status.context_name = Some("main".into());
    app.status.resolution_source = Some("配置".into());
    app.status.git_branch = Some("功能/多字节".into());
    app.status.update_context_warning();
    app.session_name = Some("会话".into());
    app
}

fn picker() -> SessionPicker {
    SessionPicker {
        sessions: vec![
            SessionPickerEntry {
                id: "abcdef1234".into(),
                name: "デモ".into(),
                turns: 5,
                cost: 0.12,
                last_active: "1m".into(),
            },
            SessionPickerEntry {
                id: "zz990000".into(),
                name: "second".into(),
                turns: 2,
                cost: 1.5,
                last_active: "now".into(),
            },
        ],
        selected: 0,
    }
}

/// Scatter `text` across the input-area rows at spaced columns, mimicking the
/// screenshot in #174 (digits at intervals, a `%` near the right margin).
///
/// Nothing in the render pipeline knows these cells changed, so only a full
/// repaint can remove them.
fn stamp_stray_glyphs(screen: &mut ScreenModel, text: &str) {
    let row = screen.height.saturating_sub(7);
    for (offset, ch) in text.chars().enumerate() {
        let x = (offset as u16 * 3) % screen.width;
        let index = row as usize * screen.width as usize + x as usize;
        screen.cells[index] = ch.to_string();
    }
}

fn suggestion_list() -> Vec<archon_tui::commands::CommandInfo> {
    vec![
        archon_tui::commands::CommandInfo {
            name: "/help".into(),
            description: "show help".into(),
            kind: archon_tui::commands::CommandKind::Primary,
        },
        archon_tui::commands::CommandInfo {
            name: "/resume".into(),
            description: "会話を再開する".into(),
            kind: archon_tui::commands::CommandKind::Primary,
        },
    ]
}

// ───────────────────────────────────────────────────────────────────────────
// Harness self-test
//
// A detector that cannot fire proves nothing, so this pins the drift the model
// exists to expose before the acceptance tests lean on it.
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn screen_model_reproduces_the_drift_a_mis_measured_glyph_causes() {
    let mut screen = ScreenModel::new(6, 1);
    let wide = Cell::new("世");
    let letter = Cell::new("A");

    // What a widget emits when it believes "世" is one column wide: three
    // cells at 0, 1 and 2, adjacent, so no cursor move is sent between them.
    screen
        .draw([(0, 0, &wide), (1, 0, &letter), (2, 0, &letter)].into_iter())
        .expect("draw");

    let row: Vec<&str> = screen.cells.iter().map(String::as_str).collect();
    assert_eq!(
        row,
        ["世", "", "A", "A", " ", " "],
        "the terminal's cursor is two columns past the wide glyph, so the \
         letters land one column right of where the widget put them"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Acceptance
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn hundred_cycles_leave_zero_stray_cells() {
    const CYCLES: usize = 100;
    const BASE_WIDTH: u16 = 100;
    const HEIGHT: u16 = 30;

    let mut app = app_with_wide_status();
    let mut terminal = Terminal::new(ScreenModel::new(BASE_WIDTH, HEIGHT)).expect("terminal");
    let mut repaint = RepaintTracker::default();

    for cycle in 0..CYCLES {
        // 1. Type a draft long enough to wrap the input area over several
        //    rows, mixing double-width text with ASCII.
        app.input.set_text(&format!(
            "{}cycle {cycle} 世界 tail",
            "wrapped 入力 draft ".repeat(6)
        ));
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "typed wrapped input");

        // 2. Submit — the input area shrinks back to its minimum height.
        app.submit_input();
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "after submit");

        // 3. Stray glyphs land in the input area — the reported symptom, and
        //    what a console write behind the renderer's back produces. The
        //    renderer cannot diff them away because it never wrote them, so
        //    the next geometry change has to repaint over them.
        stamp_stray_glyphs(terminal.backend_mut(), "392k/1000k (39%)");

        // 4. Suggestions popup: a floating element over the output area.
        app.input.set_text("/re");
        app.input.suggestions.active = true;
        app.input.suggestions.suggestions = suggestion_list();
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "suggestions open");

        app.input.suggestions.active = false;
        app.input.suggestions.suggestions.clear();
        app.input.set_text("");
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "suggestions closed");

        // 5. A centred picker, then a centred modal.
        app.session_picker = Some(picker());
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "picker open");

        app.session_picker = None;
        app.btw_overlay = Some("side question 側の質問".into());
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "picker to modal");

        app.btw_overlay = None;
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "modal closed");

        // 6. Resize by a single column, alternating direction, so consecutive
        //    cycles both grow and shrink the frame.
        let width = if cycle % 2 == 0 {
            BASE_WIDTH - 1
        } else {
            BASE_WIDTH
        };
        terminal.backend_mut().resize(width, HEIGHT);
        draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "after resize");
    }
}

#[test]
fn ctrl_l_repaints_a_screen_that_was_corrupted_behind_ratatuis_back() {
    let mut app = app_with_wide_status();
    let mut terminal = Terminal::new(ScreenModel::new(80, 24)).expect("terminal");
    let mut repaint = RepaintTracker::default();
    draw_frame(&mut terminal, &mut app, &mut repaint).expect("first draw");

    // Simulate what a stray write to the console does: cells the renderer has
    // no idea changed, so its diff will not repaint them.
    for index in 0..40 {
        terminal.backend_mut().cells[index] = "\u{2591}".to_string();
    }
    let live = terminal.backend().cells.clone();
    let fresh = freshly_rendered(&mut app, 80, 24);
    assert!(
        !stray_cells(&live, &fresh, 80).is_empty(),
        "the corruption must be visible before the escape hatch is used"
    );

    // Ctrl+L is answered ahead of the input dispatch and forces a full repaint.
    let ctrl_l = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert!(archon_tui::render::note_terminal_event(
        &ctrl_l,
        &mut repaint
    ));
    draw_and_assert_clean(&mut terminal, &mut app, &mut repaint, "after Ctrl+L");
}
