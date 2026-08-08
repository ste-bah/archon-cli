//! Startup splash screen for the Archon TUI.
//!
//! Renders a compact Archon logo alongside model info, recent activity, and
//! tips. The halfblock avatar renderer is retained for compatibility tests,
//! but the startup screen now prefers text art so it stays crisp on WSL TTYs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::splash_image::{get_avatar, render_halfblock_image};
use crate::theme::{Theme, intj_theme};

pub use crate::splash_compat::render_splash;

/// Test-only access to `render_halfblock_image` for integration tests.
#[doc(hidden)]
pub fn test_render_halfblock_image(buf: &mut Buffer, area: Rect, img: &image::DynamicImage) {
    render_halfblock_image(buf, area, img);
}

// ---------------------------------------------------------------------------
// ASCII fallback logo (retained for tiny terminals)
// ---------------------------------------------------------------------------

pub(crate) const ASCII_FALLBACK: &[&str] = &[
    "        ___  ____   ____ _   _ ",
    "       / _ \\|  _ \\ / ___| | | |",
    "      | |_| | |_) | |   | |_| |",
    "      |  _  |  _ <| |___|  _  |",
    "      |_| |_|_| \\_\\\\____|_| |_|",
    "          evidence + agents     ",
    "          memory + learning     ",
    "          codex + claude oauth  ",
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
/// the area: left column for the logo, right column for text (activity,
/// model, tips).
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
    let use_ascii_fallback = true;

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
    while activity_lines.len() < 12 {
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
        "/auth status for OAuth",
        "/agents shows activity",
        "/docs ingest <path>",
        "/gametheory run ...",
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

    // Every render below goes through `row`/`cell`, which drop anything past
    // the bottom of `area`. See their doc comments: the splash is a fixed-height
    // composition and ratatui panics rather than clipping.

    // Top border
    let version = concat!("Archon v", env!("CARGO_PKG_VERSION"));
    let dashes_after = area.width.saturating_sub(5 + version.len() as u16 + 1);
    let top_text = format!(
        "╭─── {version} {pad}╮",
        pad = "─".repeat(dashes_after as usize)
    );
    let top_para = Paragraph::new(top_text).style(Style::default().fg(t.border_active));
    if let Some(rect) = row(area, area.y) {
        top_para.render(rect, buf);
    }

    // Blank + header
    if let Some(rect) = row(area, area.y + 1) {
        bordered_paragraph("", &t, area.width).render(rect, buf);
    }

    if let Some(rect) = row(area, area.y + 2) {
        bordered_paragraph_line(&header_line, &t, area.width).render(rect, buf);
    }

    // Blank
    if let Some(rect) = row(area, area.y + 3) {
        bordered_paragraph("", &t, area.width).render(rect, buf);
    }

    // Image + activity section (rows 4-8)
    let image_area_top = area.y + 4;
    let image_area_height = 12u16;
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

            let Some(rect) = row(area, row_y) else { break };
            let line = logo_activity_line(&t, area.width as usize, logo_str, &activity_str);
            Paragraph::new(line).render(rect, buf);
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
            if row_y >= area.bottom() {
                break;
            }
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
        let Some(border_rect) = cell(area, area.right().saturating_sub(1), row_y) else {
            break;
        };
        Paragraph::new("│")
            .style(Style::default().fg(t.border_active))
            .render(border_rect, buf);
    }

    // Blank after image section
    let post_image_y = image_area_top + image_area_height;
    if let Some(rect) = row(area, post_image_y) {
        bordered_paragraph("", &t, area.width).render(rect, buf);
    }

    // Model + Tips header
    let tips_header_y = post_image_y + 1;
    if let Some(rect) = row(area, tips_header_y) {
        bordered_paragraph_line(&tips_header_line, &t, area.width).render(rect, buf);
    }

    // Tip lines
    for (i, tip_line) in tip_lines.iter().enumerate() {
        let Some(rect) = row(area, tips_header_y + 1 + i as u16) else {
            break;
        };
        bordered_paragraph_line(tip_line, &t, area.width).render(rect, buf);
    }

    // Blank
    let pre_bottom_y = tips_header_y + 1 + tip_lines.len() as u16;
    if let Some(rect) = row(area, pre_bottom_y) {
        bordered_paragraph("", &t, area.width).render(rect, buf);
    }

    // Bottom border
    let bottom_y = pre_bottom_y + 1;
    if let Some(rect) = row(area, bottom_y) {
        let bottom_inner = "─".repeat(area.width.saturating_sub(2) as usize);
        Paragraph::new(format!("╰{bottom_inner}╯"))
            .style(Style::default().fg(t.border_active))
            .render(rect, buf);
    }

    // Prompt hint
    if let Some(rect) = row(area, bottom_y + 1) {
        Paragraph::new(Span::styled(" >", Style::default().fg(t.fg))).render(rect, buf);
    }
}

/// A full-width one-row rect at `y`, or `None` when `y` is past the bottom of
/// `area`.
///
/// The splash is a fixed-height composition — border, header, twelve rows of
/// logo and activity, tips, bottom border, prompt — laid out at absolute
/// offsets from `area.y`, and it does not shrink. On a terminal shorter than
/// that composition the later offsets address rows outside the buffer, and
/// ratatui's `Buffer` panics on an out-of-bounds index rather than clipping:
/// `index outside of buffer: the area is Rect { .. height: 15 } but index is
/// (0, 15)`.
///
/// Clipping here rather than laying the splash out dynamically is deliberate.
/// The art has a fixed shape; a short terminal should lose the bottom of it,
/// which is decoration, and keep the top, which names the version and the
/// working directory. What it must not do is take the process down.
fn row(area: Rect, y: u16) -> Option<Rect> {
    (y < area.bottom()).then(|| Rect::new(area.x, y, area.width, 1))
}

/// One cell at `(x, y)`, or `None` when it falls outside `area`. Same reasoning
/// as [`row`]; used for the right-hand border, which is drawn a cell at a time.
fn cell(area: Rect, x: u16, y: u16) -> Option<Rect> {
    (y < area.bottom() && x < area.right()).then(|| Rect::new(x, y, 1, 1))
}

// render_splash() is in splash_compat.rs — re-exported below for backward compat.

pub fn logo_activity_line<'a>(t: &Theme, width: usize, logo: &str, activity: &str) -> Line<'a> {
    let half = width / 2;
    let col = half.saturating_sub(1);
    // Padded but not truncated, an over-long activity description ran past the
    // column, overwrote the closing border, and was cut off by the frame edge
    // mid-word with nothing to say it had been. `fit` makes the cut visible.
    // The logo is fixed art the layout is sized around, so it is left alone.
    let left_padded = format!("{:<w$}", logo, w = col);
    let right_padded = format!("{:<w$}", fit(activity, col), w = col);
    Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::styled(left_padded, Style::default().fg(t.header)),
        Span::styled(right_padded, Style::default().fg(t.muted)),
        Span::styled("│", Style::default().fg(t.border_active)),
    ])
}

/// Shorten `s` to `max` display columns, ending in "..." when it had to cut.
///
/// Char-boundary safe. Below four columns there is no room for the marker, so
/// it degrades to a hard cut rather than returning something wider than asked
/// for -- a splash column is a hard limit, not a preference.
fn fit(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    if max <= 3 {
        return s.chars().take(max).collect();
    }
    let kept: String = s.chars().take(max - 3).collect();
    format!("{kept}...")
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

#[cfg(test)]
#[path = "splash_tests.rs"]
mod tests;
