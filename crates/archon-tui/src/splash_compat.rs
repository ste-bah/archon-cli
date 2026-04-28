//! Backward-compatible `render_splash()` that returns `Vec<Line>`.
//!
//! New callers should use `splash::draw_splash()` (the `&mut Buffer` API).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::splash::{ASCII_FALLBACK, ActivityEntry, logo_activity_line, truncate_path};
use crate::theme::{Theme, intj_theme};

/// Render the splash screen as a vector of styled `Line`s.
///
/// Kept for backward compatibility. New callers should prefer
/// `splash::draw_splash()`.
pub fn render_splash<'a>(
    model: &str,
    working_dir: &str,
    activity: &[ActivityEntry],
) -> Vec<Line<'a>> {
    let t = intj_theme();
    let width: usize = 64;

    let mut lines: Vec<Line<'a>> = Vec::with_capacity(20);

    lines.push(top_border(&t, width));

    lines.push(bordered_blank(&t, width));

    lines.push(two_column(
        &t,
        width,
        (
            "   Welcome back!",
            Style::default().fg(t.header).add_modifier(Modifier::BOLD),
        ),
        (
            "Recent Activity",
            Style::default()
                .fg(t.accent_secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ));

    lines.push(bordered_blank(&t, width));

    let max_activity = 3;
    for (i, logo_line) in ASCII_FALLBACK.iter().enumerate() {
        let right = if i < activity.len().min(max_activity) {
            let a = &activity[i];
            format!("{:<8} {}", a.when, a.description)
        } else if i == max_activity && activity.len() > max_activity {
            ".../resume for more".to_owned()
        } else {
            String::new()
        };

        lines.push(logo_activity_line(&t, width, logo_line, &right));
    }

    lines.push(bordered_blank(&t, width));

    lines.push(two_column(
        &t,
        width,
        (&format!("   {model}"), Style::default().fg(t.accent)),
        (
            "Tips",
            Style::default()
                .fg(t.accent_secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ));

    let tips = [
        "/model to switch models",
        "/help for all commands",
        "Type ultrathink for deep",
        "... /help for more",
    ];

    let dir_display = truncate_path(working_dir, 24);
    for (i, tip) in tips.iter().enumerate() {
        let left = if i == 0 {
            format!("   {dir_display}")
        } else {
            String::new()
        };
        lines.push(two_column(
            &t,
            width,
            (&left, Style::default().fg(t.muted)),
            (tip, Style::default().fg(t.muted)),
        ));
    }

    lines.push(bordered_blank(&t, width));

    lines.push(bottom_border(&t, width));

    lines.push(Line::from(Span::styled(" >", Style::default().fg(t.fg))));

    lines
}

// ---------------------------------------------------------------------------
// Helpers used only by render_splash
// ---------------------------------------------------------------------------

fn top_border<'a>(t: &Theme, width: usize) -> Line<'a> {
    let version = concat!("Archon v", env!("CARGO_PKG_VERSION"));
    let dashes_after = width.saturating_sub(5 + version.len() + 1);
    let text = format!("╭─── {version} {pad}╮", pad = "─".repeat(dashes_after));
    Line::from(Span::styled(text, Style::default().fg(t.border_active)))
}

fn bottom_border<'a>(t: &Theme, width: usize) -> Line<'a> {
    let inner = "─".repeat(width.saturating_sub(2));
    let text = format!("╰{inner}╯");
    Line::from(Span::styled(text, Style::default().fg(t.border_active)))
}

fn bordered_blank<'a>(t: &Theme, width: usize) -> Line<'a> {
    let inner = " ".repeat(width.saturating_sub(2));
    Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::raw(inner),
        Span::styled("│", Style::default().fg(t.border_active)),
    ])
}

fn two_column<'a>(t: &Theme, width: usize, left: (&str, Style), right: (&str, Style)) -> Line<'a> {
    let half = width / 2;
    let left_padded = format!("{:<w$}", left.0, w = half.saturating_sub(1));
    let right_padded = format!("{:<w$}", right.0, w = half.saturating_sub(1));
    Line::from(vec![
        Span::styled("│", Style::default().fg(t.border_active)),
        Span::styled(left_padded, left.1),
        Span::styled(right_padded, right.1),
        Span::styled("│", Style::default().fg(t.border_active)),
    ])
}
