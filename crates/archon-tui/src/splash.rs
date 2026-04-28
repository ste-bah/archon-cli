//! Startup splash screen for the Archon TUI.
//!
//! Renders the archon-avatar.png pixel art using unicode halfblock characters
//! alongside model info, recent activity, and tips. Falls back to ASCII box
//! art on tiny terminals where the image would be illegible.

use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::theme::{Theme, intj_theme};

pub use crate::splash_compat::render_splash;

// ---------------------------------------------------------------------------
// Embedded avatar
// ---------------------------------------------------------------------------

const AVATAR_PNG: &[u8] = include_bytes!("../../../archon-avatar.png");

// ---------------------------------------------------------------------------
// Cached decoded image (decoded once, reused across frames)
// ---------------------------------------------------------------------------

static AVATAR_IMAGE: OnceLock<image::DynamicImage> = OnceLock::new();

fn get_avatar() -> &'static image::DynamicImage {
    AVATAR_IMAGE.get_or_init(|| {
        image::load_from_memory(AVATAR_PNG)
            .expect("archon-avatar.png must be a valid PNG at compile time")
    })
}

// ---------------------------------------------------------------------------
// Halfblock image renderer
// ---------------------------------------------------------------------------

/// Render an image into a rectangular region using unicode halfblock characters.
///
/// Each terminal cell covers 2 vertical pixels: the top pixel becomes the
/// foreground color, the bottom pixel becomes the background color, and the
/// glyph is `▀` (U+2580 UPPER HALF BLOCK).
fn render_halfblock_image(buf: &mut Buffer, area: Rect, img: &image::DynamicImage) {
    let cell_w = area.width as u32;
    let cell_h = area.height as u32;
    if cell_w == 0 || cell_h == 0 {
        return;
    }

    let pixel_w = cell_w;
    let pixel_h = cell_h * 2; // 2 rows of pixels per terminal cell

    let resized = img.resize_exact(pixel_w, pixel_h, image::imageops::FilterType::Nearest);
    let rgba = resized.to_rgba8();

    for cell_row in 0..cell_h {
        let top_y = cell_row * 2;
        let bottom_y = top_y + 1;
        let y = area.y + cell_row as u16;

        let mut spans = Vec::with_capacity(cell_w as usize);
        for col in 0..cell_w {
            let top = rgba.get_pixel(col, top_y);
            let bottom = rgba.get_pixel(col, bottom_y);
            let fg = Color::Rgb(top[0], top[1], top[2]);
            let bg = Color::Rgb(bottom[0], bottom[1], bottom[2]);
            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }

        let line = Line::from(spans);
        let row_area = Rect::new(area.x, y, area.width, 1);
        Paragraph::new(line).render(row_area, buf);
    }
}

// ---------------------------------------------------------------------------
// ASCII fallback logo (retained for tiny terminals)
// ---------------------------------------------------------------------------

pub(crate) const ASCII_FALLBACK: &[&str] = &[
    "      ╔═══╗        ",
    "      ║ ◈ ║        ",
    "    ╔═╩═══╩═╗      ",
    "    ║ ARCHON ║      ",
    "    ╚════════╝      ",
];

// ---------------------------------------------------------------------------
// Activity entry
// ---------------------------------------------------------------------------

/// A recent-activity entry shown on the splash screen.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    /// Human-readable relative time, e.g. "2h ago".
    pub when: String,
    /// Short description, e.g. "Chat session".
    pub description: String,
}

/// Format an RFC3339 timestamp as a human-readable relative time string.
pub fn format_relative_time(rfc3339: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return "unknown".to_string();
    };
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(then);

    let secs = duration.num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = duration.num_minutes();
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = duration.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = duration.num_days();
    if days < 30 {
        return format!("{days}d ago");
    }
    if days < 365 {
        let months = days / 30;
        return format!("{months}mo ago");
    }
    let years = days / 365;
    format!("{years}yr ago")
}

// ---------------------------------------------------------------------------
// Main draw function — renders directly into a ratatui Frame
// ---------------------------------------------------------------------------

/// Render the splash screen directly into a ratatui `Frame`.
///
/// The caller (body.rs) passes the full output area. This function splits
/// the area: left column for the avatar image, right column for text
/// (activity, model, tips).
pub fn draw_splash(
    buf: &mut Buffer,
    area: Rect,
    model: &str,
    working_dir: &str,
    activity: &[ActivityEntry],
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let t = intj_theme();
    let use_ascii_fallback = area.width < 40;

    let inner_w = area.width.saturating_sub(2) as usize;
    let half = inner_w / 2;

    // ── Build lines for the text-column Paragraphs ────────────────

    // Welcome + Recent Activity header
    let header_left = format!("{:<w$}", " Welcome back!", w = half);
    let header_right = format!("{:<w$}", "Recent Activity", w = half);
    let header_line = Line::from(vec![
        Span::styled(
            header_left,
            Style::default().fg(t.header).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            header_right,
            Style::default()
                .fg(t.accent_secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    // Activity lines (right half, shown next to image)
    let max_activity = 3;
    let mut activity_lines: Vec<Line<'_>> = Vec::with_capacity(max_activity + 1);
    for (_i, a) in activity.iter().enumerate().take(max_activity) {
        activity_lines.push(Line::from(Span::styled(
            format!("{:<8} {}", a.when, a.description),
            Style::default().fg(t.muted),
        )));
    }
    if activity.len() > max_activity {
        activity_lines.push(Line::from(Span::styled(
            ".../resume for more",
            Style::default().fg(t.muted),
        )));
    }
    while activity_lines.len() < 5 {
        activity_lines.push(Line::from(""));
    }

    // Model + Tips header
    let tips_header_left = format!("{:<w$}", format!("   {model}"), w = half);
    let tips_header_right = format!("{:<w$}", "Tips", w = half);
    let tips_header_line = Line::from(vec![
        Span::styled(tips_header_left, Style::default().fg(t.accent)),
        Span::styled(
            tips_header_right,
            Style::default()
                .fg(t.accent_secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    // Working dir + tip lines
    let tips = [
        "/model to switch models",
        "/help for all commands",
        "Type ultrathink for deep",
        "... /help for more",
    ];
    let dir_display = truncate_path(working_dir, 24);
    let mut tip_lines: Vec<Line<'_>> = Vec::with_capacity(4);
    for (i, tip) in tips.iter().enumerate() {
        let left = if i == 0 {
            format!("   {dir_display}")
        } else {
            String::new()
        };
        let left_padded = format!("{:<w$}", left, w = half);
        let right_padded = format!("{:<w$}", tip, w = half);
        tip_lines.push(Line::from(vec![
            Span::styled(left_padded, Style::default().fg(t.muted)),
            Span::styled(right_padded, Style::default().fg(t.muted)),
        ]));
    }

    // ── Render into the frame ────────────────────────────────────

    // Top border
    let version = concat!("Archon v", env!("CARGO_PKG_VERSION"));
    let dashes_after = area.width.saturating_sub(5 + version.len() as u16 + 1);
    let top_text = format!(
        "╭─── {version} {pad}╮",
        pad = "─".repeat(dashes_after as usize)
    );
    let top_para = Paragraph::new(top_text).style(Style::default().fg(t.border_active));
    let top_area = Rect::new(area.x, area.y, area.width, 1);
    top_para.render(top_area, buf);

    // Blank + header
    let row1_y = area.y + 1;
    let blank1 = bordered_paragraph("", &t, area.width);
    blank1.render(Rect::new(area.x, row1_y, area.width, 1), buf);

    let row2_y = area.y + 2;
    bordered_paragraph_line(&header_line, &t, area.width)
        .render(Rect::new(area.x, row2_y, area.width, 1), buf);

    // Blank
    let row3_y = area.y + 3;
    bordered_paragraph("", &t, area.width).render(Rect::new(area.x, row3_y, area.width, 1), buf);

    // Image + activity section (rows 4-8)
    let image_area_top = area.y + 4;
    let image_area_height = 5u16;
    let left_col_x = area.x + 1;

    if use_ascii_fallback {
        for i in 0..image_area_height {
            let row_y = image_area_top + i;
            let idx = i as usize;
            let logo_str = ASCII_FALLBACK.get(idx).copied().unwrap_or("");
            let activity_str = activity_lines
                .get(idx)
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .unwrap_or_default();

            let line = logo_activity_line(&t, area.width as usize, logo_str, &activity_str);
            let para = Paragraph::new(line);
            para.render(Rect::new(area.x, row_y, area.width, 1), buf);
        }
    } else {
        let image_width = (half as u16).min(area.width.saturating_sub(2));
        let image_rect = Rect::new(left_col_x, image_area_top, image_width, image_area_height);
        render_halfblock_image(buf, image_rect, get_avatar());

        // Render activity entries on the right half
        let right_col_x = left_col_x + image_width;
        let right_width = area.width.saturating_sub(2).saturating_sub(image_width);
        for (i, line) in activity_lines
            .iter()
            .enumerate()
            .take(image_area_height as usize)
        {
            let row_y = image_area_top + i as u16;
            let right_rect = Rect::new(right_col_x, row_y, right_width, 1);
            let para = Paragraph::new(line.clone())
                .style(Style::default().fg(t.muted))
                .wrap(Wrap { trim: false });
            para.render(right_rect, buf);
        }
    }

    // Right border for image/activity rows
    for i in 0..image_area_height {
        let row_y = image_area_top + i;
        let border_rect = Rect::new(area.right().saturating_sub(1), row_y, 1, 1);
        Paragraph::new("│")
            .style(Style::default().fg(t.border_active))
            .render(border_rect, buf);
    }

    // Blank after image section
    let post_image_y = image_area_top + image_area_height;
    bordered_paragraph("", &t, area.width)
        .render(Rect::new(area.x, post_image_y, area.width, 1), buf);

    // Model + Tips header
    let tips_header_y = post_image_y + 1;
    bordered_paragraph_line(&tips_header_line, &t, area.width)
        .render(Rect::new(area.x, tips_header_y, area.width, 1), buf);

    // Tip lines
    for (i, tip_line) in tip_lines.iter().enumerate() {
        let row_y = tips_header_y + 1 + i as u16;
        bordered_paragraph_line(tip_line, &t, area.width)
            .render(Rect::new(area.x, row_y, area.width, 1), buf);
    }

    // Blank
    let pre_bottom_y = tips_header_y + 1 + tip_lines.len() as u16;
    bordered_paragraph("", &t, area.width)
        .render(Rect::new(area.x, pre_bottom_y, area.width, 1), buf);

    // Bottom border
    let bottom_y = pre_bottom_y + 1;
    let bottom_inner = "─".repeat(area.width.saturating_sub(2) as usize);
    let bottom_text = format!("╰{bottom_inner}╯");
    Paragraph::new(bottom_text)
        .style(Style::default().fg(t.border_active))
        .render(Rect::new(area.x, bottom_y, area.width, 1), buf);

    // Prompt hint
    let prompt_y = bottom_y + 1;
    Paragraph::new(Span::styled(" >", Style::default().fg(t.fg)))
        .render(Rect::new(area.x, prompt_y, area.width, 1), buf);
}

// render_splash() is in splash_compat.rs — re-exported below for backward compat.

pub fn logo_activity_line<'a>(t: &Theme, width: usize, logo: &str, activity: &str) -> Line<'a> {
    let half = width / 2;
    let left_padded = format!("{:<w$}", logo, w = half.saturating_sub(1));
    let right_padded = format!("{:<w$}", activity, w = half.saturating_sub(1));
    Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::styled(left_padded, Style::default().fg(t.header)),
        Span::styled(right_padded, Style::default().fg(t.muted)),
        Span::styled("│", Style::default().fg(t.border_active)),
    ])
}

pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_owned();
    }
    let suffix = &path[path.len() - (max_len - 3)..];
    format!("...{suffix}")
}

/// Wrap a single-line string in vertical border characters.
fn bordered_paragraph<'a>(text: &str, t: &Theme, width: u16) -> Paragraph<'a> {
    let inner_w = width.saturating_sub(2) as usize;
    let padded = format!("{:<w$}", text, w = inner_w);
    let line = Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::raw(padded),
        Span::styled("│", Style::default().fg(t.border_active)),
    ]);
    Paragraph::new(line)
}

/// Wrap a ratatui Line in vertical border characters.
fn bordered_paragraph_line<'a>(line: &Line<'a>, t: &Theme, width: u16) -> Paragraph<'a> {
    let inner_w = width.saturating_sub(2) as usize;
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let padded = format!("{:<w$}", text, w = inner_w);
    Paragraph::new(Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::raw(padded),
        Span::styled("│", Style::default().fg(t.border_active)),
    ]))
}
