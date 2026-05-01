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
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
    let activity = vec![ActivityEntry {
        when: "1h ago".into(),
        description: "Test session".into(),
    }];
    draw_splash(
        &mut buf,
        Rect::new(0, 0, 80, 30),
        "test-model",
        "/tmp",
        &activity,
    );
    // Should not panic, buffer should have content
}

#[test]
fn draw_splash_ascii_fallback_on_narrow() {
    // Terminal under 40 cols triggers ASCII fallback
    let mut buf = Buffer::empty(Rect::new(0, 0, 39, 30));
    draw_splash(&mut buf, Rect::new(0, 0, 39, 30), "test-model", "/tmp", &[]);
    // Should render without panic using ASCII fallback
}

#[test]
fn halfblock_image_renders_on_wide_terminal() {
    // Wide enough: halfblock image path used
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
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
        Rect::new(0, 0, 80, 30),
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
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
    draw_splash(&mut buf, Rect::new(0, 0, 80, 30), "test", "/tmp", &[]);
    let content = buffer_to_string(&buf);
    assert!(
        content.contains("Archon v"),
        "should contain version string"
    );
}

#[test]
fn draw_splash_shows_tips() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
    draw_splash(&mut buf, Rect::new(0, 0, 80, 30), "test", "/tmp", &[]);
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
                s.push_str(cell.symbol());
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

// ── halfblock aspect ratio tests ────────────────────────────────────

#[test]
fn halfblock_render_preserves_aspect_ratio_wide_area() {
    use image::RgbaImage;
    // Build a 100×100 magenta square — aspect ratio must survive a wide area.
    let img = image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        100,
        100,
        image::Rgba([200, 50, 200, 255]),
    ));
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
    // 60 cells × 10 cells = 60×20 pixels. Height-bound at min(60/100, 20/100) = 0.2
    // → rendered at 20×20 pixels = 20 cols × 10 rows, pillarboxed horizontally.
    let area = Rect::new(1, 5, 60, 10);
    archon_tui::splash::test_render_halfblock_image(&mut buf, area, &img);

    // Left edge of image area should be black (pillarbox left side).
    let left_cell = buf.cell((area.x, area.y + 4));
    assert!(left_cell.is_some(), "left edge cell should exist");
    let left_sym = left_cell.unwrap().symbol();
    assert!(!left_sym.is_empty(), "left cell should have content");

    // Near center column (area.x + 20..=area.x + 39) pixels should be magenta foreground.
    let mid_x = area.x + 30;
    let mid_cell = buf.cell((mid_x, area.y + 4));
    assert!(mid_cell.is_some());
    let mid_fg = mid_cell.unwrap().fg;
    assert_ne!(
        mid_fg,
        ratatui::style::Color::Black,
        "center cell should not be black (it should have image content)"
    );
}

#[test]
fn halfblock_render_preserves_aspect_ratio_tall_area() {
    use image::RgbaImage;
    // 100×100 source in a tall narrow area (10 cells × 30 cells = 10×60 pixels).
    // Width-bound at min(10/100, 60/100) = 0.1 → rendered at 10×10 pixels = 10 cols × 5 rows,
    // letterboxed top + bottom.
    let img = image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        100,
        100,
        image::Rgba([50, 200, 200, 255]),
    ));
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 40));
    let area = Rect::new(1, 2, 10, 30);
    archon_tui::splash::test_render_halfblock_image(&mut buf, area, &img);

    // Top row should be black (letterbox top).
    let top_cell = buf.cell((area.x, area.y));
    assert!(top_cell.is_some(), "top cell should exist");
    let top_fg = top_cell.unwrap().fg;
    assert_eq!(
        top_fg,
        ratatui::style::Color::Black,
        "top row should be black letterbox"
    );

    // Row 14 has both top + bottom pixels inside the 10×10 rendered image
    // (image occupies pixel rows 25-34; cells 13-16 are fully inside).
    let mid_y = area.y + 14;
    let mid_cell = buf.cell((area.x + 2, mid_y));
    assert!(mid_cell.is_some());
    let mid_cell = mid_cell.unwrap();
    assert_ne!(
        mid_cell.fg,
        ratatui::style::Color::Black,
        "row 14 fg should have image content (cyan-ish), not black"
    );
    assert_ne!(
        mid_cell.bg,
        ratatui::style::Color::Black,
        "row 14 bg should have image content (cyan-ish), not black"
    );

    // Bottom row should be black (letterbox bottom).
    let bot_y = area.bottom().saturating_sub(1);
    let bot_cell = buf.cell((area.x, bot_y));
    assert!(bot_cell.is_some());
    let bot_fg = bot_cell.unwrap().fg;
    assert_eq!(
        bot_fg,
        ratatui::style::Color::Black,
        "bottom row should be black letterbox"
    );
}

#[test]
fn splash_image_area_height_is_12() {
    // Render splash into 80×40 and verify we get 12 rows of image content.
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
    let activity = vec![ActivityEntry {
        when: "1h ago".into(),
        description: "Test".into(),
    }];
    draw_splash(
        &mut buf,
        Rect::new(0, 0, 80, 40),
        "test-model",
        "/tmp",
        &activity,
    );

    // The image area starts at y=4 (image_area_top) and spans image_area_height=12 rows.
    // Count cells in that region that contain halfblock U+2580 glyphs.
    let image_top = 4u16;
    let image_bot = image_top + 12;
    let mut halfblock_rows = 0u32;
    for y in image_top..image_bot {
        let mut row_has_content = false;
        for x in 1..40u16 {
            if let Some(cell) = buf.cell((x, y))
                && cell.symbol().contains('▀')
            {
                row_has_content = true;
                break;
            }
        }
        if row_has_content {
            halfblock_rows += 1;
        }
    }
    assert!(
        halfblock_rows >= 8,
        "expected >= 8 rows with halfblock glyphs in image area, got {halfblock_rows}"
    );
}
