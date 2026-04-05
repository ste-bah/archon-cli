//! Startup splash screen for the Archon TUI.
//!
//! Renders a Claude-Code-style welcome panel with ASCII art, model info,
//! recent activity, and tips — all in the INTJ color scheme.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Theme, intj_theme};

/// A recent-activity entry shown on the splash screen.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    /// Human-readable relative time, e.g. "2h ago".
    pub when: String,
    /// Short description, e.g. "Chat session".
    pub description: String,
}

/// Format an RFC3339 timestamp as a human-readable relative time string.
/// Returns e.g. "just now", "5m ago", "2h ago", "3d ago".
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

/// Render the splash screen as a vector of styled `Line`s.
///
/// * `model` — current model name (e.g. `"claude-sonnet-4-6"`).
/// * `working_dir` — working directory path to display.
/// * `activity` — recent session entries (at most 3 are shown).
pub fn render_splash<'a>(
    model: &str,
    working_dir: &str,
    activity: &[ActivityEntry],
) -> Vec<Line<'a>> {
    let t = intj_theme();
    let width: usize = 64;

    let mut lines: Vec<Line<'a>> = Vec::with_capacity(20);

    // ── top border ───────────────────────────────────────────────
    lines.push(top_border(&t, width));

    // blank
    lines.push(bordered_blank(&t, width));

    // "Welcome back!" + "Recent Activity" header
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

    // blank
    lines.push(bordered_blank(&t, width));

    // ASCII logo lines (left) + activity entries (right)
    let logo = [
        "      ╔═══╗        ",
        "      ║ ◈ ║        ",
        "    ╔═╩═══╩═╗      ",
        "    ║ ARCHON ║      ",
        "    ╚════════╝      ",
    ];

    let max_activity = 3;
    for (i, logo_line) in logo.iter().enumerate() {
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

    // blank
    lines.push(bordered_blank(&t, width));

    // Model + Tips header
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

    // Working dir + tip lines
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

    // blank
    lines.push(bordered_blank(&t, width));

    // bottom border
    lines.push(bottom_border(&t, width));

    // prompt hint
    lines.push(Line::from(Span::styled(" >", Style::default().fg(t.fg))));

    lines
}

// ── helper builders ──────────────────────────────────────────────

fn top_border<'a>(t: &Theme, width: usize) -> Line<'a> {
    let version = "Archon v0.1.0";
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

fn logo_activity_line<'a>(t: &Theme, width: usize, logo: &str, activity: &str) -> Line<'a> {
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

fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_owned();
    }
    let suffix = &path[path.len() - (max_len - 3)..];
    format!("...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
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
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
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
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
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
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(text.contains("ARCHON"), "splash should contain ASCII logo");
    }

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
}
