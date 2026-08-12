//! Unit tests for the full-repaint policy (`render::repaint`).

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::App;

use super::{RepaintTracker, frame_geometry, note_terminal_event};

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn app_without_splash() -> App {
    let mut app = App::new();
    app.show_splash = false;
    app
}

/// Advance the tracker by one frame and report whether it asked for a clear.
fn frame(tracker: &mut RepaintTracker, app: &App, area: Rect) -> bool {
    tracker.needs_clear(frame_geometry(app, area))
}

fn ctrl(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
}

#[test]
fn first_frame_does_not_clear() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();

    assert!(
        !frame(&mut tracker, &app, AREA),
        "the opening frame writes every cell without help"
    );
}

#[test]
fn steady_geometry_does_not_clear() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();

    frame(&mut tracker, &app, AREA);
    assert!(!frame(&mut tracker, &app, AREA));
    assert!(!frame(&mut tracker, &app, AREA));
}

#[test]
fn opening_and_closing_an_overlay_each_clear() {
    let mut app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    app.btw_overlay = Some("side question".to_string());
    assert!(frame(&mut tracker, &app, AREA), "overlay open must clear");
    assert!(!frame(&mut tracker, &app, AREA), "then settle");

    app.btw_overlay = None;
    assert!(frame(&mut tracker, &app, AREA), "overlay close must clear");
}

#[test]
fn input_area_shrinking_after_submit_clears() {
    let mut app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    // A draft long enough to wrap the input area past its minimum height.
    app.input.set_text(&"wrapped draft text ".repeat(30));
    let grown = frame_geometry(&app, AREA);
    assert!(frame(&mut tracker, &app, AREA), "growth must clear");

    app.submit_input();
    let shrunk = frame_geometry(&app, AREA);
    assert_ne!(
        grown, shrunk,
        "submitting must actually change the input geometry"
    );
    assert!(
        frame(&mut tracker, &app, AREA),
        "shrinking back to one row must clear"
    );
}

#[test]
fn size_change_defers_to_ratatui_autoresize() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    let narrower = Rect { width: 79, ..AREA };
    assert!(
        !frame(&mut tracker, &app, narrower),
        "ratatui clears on its own when the reported size changed; a second \
         clear would repaint every step of a drag-resize twice"
    );
}

#[test]
fn forced_repaint_clears_exactly_one_frame() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    tracker.request_full_repaint();
    assert!(frame(&mut tracker, &app, AREA));
    assert!(
        !frame(&mut tracker, &app, AREA),
        "the forced flag must not stick"
    );
}

#[test]
fn same_size_resize_notification_still_clears() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    assert!(!note_terminal_event(
        &Event::Resize(AREA.width, AREA.height),
        &mut tracker
    ));
    assert!(
        frame(&mut tracker, &app, AREA),
        "ratatui skips a resize that did not change the size, so we must not"
    );
}

#[test]
fn ctrl_l_is_consumed_and_forces_a_repaint() {
    let app = app_without_splash();
    let mut tracker = RepaintTracker::default();
    frame(&mut tracker, &app, AREA);

    assert!(
        note_terminal_event(&ctrl(KeyCode::Char('l')), &mut tracker),
        "Ctrl+L must not reach the input dispatch"
    );
    assert!(frame(&mut tracker, &app, AREA));
}

#[test]
fn ctrl_l_release_and_other_chords_are_left_alone() {
    let mut tracker = RepaintTracker::default();

    let release = Event::Key(KeyEvent::new_with_kind_and_state(
        KeyCode::Char('l'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
        KeyEventState::NONE,
    ));
    assert!(!note_terminal_event(&release, &mut tracker));
    assert!(!note_terminal_event(
        &ctrl(KeyCode::Char('k')),
        &mut tracker
    ));
    assert!(!note_terminal_event(
        &Event::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
        &mut tracker
    ));

    let app = app_without_splash();
    frame(&mut tracker, &app, AREA);
    assert!(
        !frame(&mut tracker, &app, AREA),
        "none of those may have armed a repaint"
    );
}
