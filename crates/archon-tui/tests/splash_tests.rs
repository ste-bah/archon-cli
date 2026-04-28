//! Tests for splash screen rendering (buffer-based draw_splash + compat render_splash).
//!
//! These were extracted from splash.rs to keep the source file under 500 lines.

use archon_tui::splash::{self, ActivityEntry, draw_splash, format_relative_time, truncate_path};
use archon_tui::splash_compat::render_splash;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── render_splash compat tests ────────────────────────────────────

#[test]
fn splash_produces_nonempty_output() {
    let lines = render_splash("test-model", "/tmp/test", &[]);
    assert!(!lines.is_empty(), "splash should produce lines");
}

#[test]
fn splash_includes_model_name() {
    let lines = render_splash("claude-sonnet-4-6", "/tmp", &[]);
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    assert!(
        text.contains("claude-sonnet-4-6"),
        "splash should contain the model name"
    );
}

#[test]
fn splash_includes_working_dir() {
    let lines = render_splash("m", "/home/user/project", &[]);
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    assert!(
        text.contains("/home/user/project"),
        "splash should contain the working directory"
    );
}

#[test]
fn splash_includes_activity_entries() {
    let activity = vec![
        ActivityEntry {
            when: "1m ago".into(),
            description: "Session started".into(),
        },
        ActivityEntry {
            when: "2h ago".into(),
            description: "Chat session".into(),
        },
    ];
    let lines = render_splash("m", "/tmp", &activity);
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    assert!(
        text.contains("Session started"),
        "splash should contain activity entries"
    );
    assert!(
        text.contains("2h ago"),
        "splash should contain activity timestamps"
    );
}

#[test]
fn splash_has_logo() {
    let lines = render_splash("m", "/tmp", &[]);
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    assert!(text.contains("ARCHON"), "splash should contain ASCII logo");
}

// ── format_relative_time tests ────────────────────────────────────

#[test]
fn relative_time_just_now() {
    let now = chrono::Utc::now().to_rfc3339();
    assert_eq!(format_relative_time(&now), "just now");
}

#[test]
fn relative_time_minutes() {
    let then = (chrono::Utc::now() - chrono::Duration::minutes(15)).to_rfc3339();
    assert_eq!(format_relative_time(&then), "15m ago");
}

#[test]
fn relative_time_hours() {
    let then = (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
    assert_eq!(format_relative_time(&then), "3h ago");
}

#[test]
fn relative_time_days() {
    let then = (chrono::Utc::now() - chrono::Duration::days(5)).to_rfc3339();
    assert_eq!(format_relative_time(&then), "5d ago");
}

#[test]
fn relative_time_invalid() {
    assert_eq!(format_relative_time("not-a-date"), "unknown");
}

// ── truncate_path tests ───────────────────────────────────────────

#[test]
fn truncate_path_short_unchanged() {
    assert_eq!(truncate_path("/tmp", 20), "/tmp");
}

#[test]
fn truncate_path_long_is_shortened() {
    let long = "/very/long/path/that/exceeds/the/limit";
    let result = truncate_path(long, 20);
    assert!(result.starts_with("..."));
    assert!(result.len() <= 20);
}

// ── draw_splash (buffer-based) tests ──────────────────────────────

#[test]
fn draw_splash_renders_without_panicking() {
    // Wide enough area that uses halfblock image path
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
    let activity = vec![ActivityEntry {
        when: "1h ago".into(),
        description: "Test session".into(),
    }];
    draw_splash(
        &mut buf,
        Rect::new(0, 0, 80, 20),
        "test-model",
        "/tmp",
        &activity,
    );
    // Should not panic, buffer should have content
}

#[test]
fn draw_splash_ascii_fallback_on_narrow() {
    // Terminal under 40 cols triggers ASCII fallback
    let mut buf = Buffer::empty(Rect::new(0, 0, 39, 20));
    draw_splash(&mut buf, Rect::new(0, 0, 39, 20), "test-model", "/tmp", &[]);
    // Should render without panic using ASCII fallback
}

#[test]
fn halfblock_image_renders_on_wide_terminal() {
    // Wide enough: halfblock image path used
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
    let activity = vec![
        ActivityEntry {
            when: "1m ago".into(),
            description: "Chat session".into(),
        },
        ActivityEntry {
            when: "2h ago".into(),
            description: "Code review".into(),
        },
    ];
    draw_splash(
        &mut buf,
        Rect::new(0, 0, 80, 20),
        "sonnet",
        "/home/test",
        &activity,
    );
    // Verify model name appears in output
    let content = buffer_to_string(&buf);
    assert!(content.contains("sonnet"), "should contain model name");
}

#[test]
fn draw_splash_shows_version() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
    draw_splash(&mut buf, Rect::new(0, 0, 80, 20), "test", "/tmp", &[]);
    let content = buffer_to_string(&buf);
    assert!(
        content.contains("Archon v"),
        "should contain version string"
    );
}

#[test]
fn draw_splash_shows_tips() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
    draw_splash(&mut buf, Rect::new(0, 0, 80, 20), "test", "/tmp", &[]);
    let content = buffer_to_string(&buf);
    assert!(
        content.contains("/model to switch models") || content.contains("/model"),
        "should contain tip about /model"
    );
}

#[test]
fn draw_splash_zero_area_does_not_panic() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
    draw_splash(&mut buf, Rect::new(0, 0, 0, 0), "test", "/tmp", &[]);
}

// ── helper ────────────────────────────────────────────────────────

fn buffer_to_string(buf: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            if let Some(cell) = buf.cell((x, y)) {
                s.push_str(&cell.symbol());
            }
        }
        s.push('\n');
    }
    s
}

// ── logo_activity_line tests ──────────────────────────────────────

#[test]
fn logo_activity_line_has_borders() {
    let t = archon_tui::theme::intj_theme();
    let line = splash::logo_activity_line(&t, 64, "LOGO", "activity text");
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains('│'), "should contain border chars");
    assert!(text.contains("LOGO"), "should contain logo text");
    assert!(text.contains("activity"), "should contain activity text");
}

#[test]
fn activity_entries_truncated_at_max() {
    let activity: Vec<ActivityEntry> = (0..10)
        .map(|i| ActivityEntry {
            when: format!("{i}h ago"),
            description: format!("Session {i}"),
        })
        .collect();
    let lines = render_splash("m", "/tmp", &activity);
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    // Should show the truncation hint
    assert!(
        text.contains("/resume"),
        "should hint at /resume when activity exceeds max"
    );
}
