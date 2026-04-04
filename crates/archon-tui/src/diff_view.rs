//! TUI diff rendering with color highlighting.
//!
//! Renders unified diff output into styled ratatui Lines with:
//! - Green `+` lines for additions
//! - Red `-` lines for deletions
//! - Gray context lines
//! - Cyan `@@` hunk headers

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render a unified diff string into colored ratatui Lines.
///
/// Lines starting with `+` are green, `-` are red, `@@` are cyan bold,
/// `---`/`+++` file headers are blue bold, and context lines are default.
pub fn render_diff(diff_text: &str) -> Vec<Line<'static>> {
    diff_text
        .lines()
        .map(|line| {
            let (style, prefix) = classify_diff_line(line);
            if prefix {
                // Color just the prefix character, rest is default
                let (pfx, rest) = line.split_at(1.min(line.len()));
                Line::from(vec![
                    Span::styled(pfx.to_string(), style),
                    Span::raw(rest.to_string()),
                ])
            } else {
                Line::from(Span::styled(line.to_string(), style))
            }
        })
        .collect()
}

/// Classify a diff line and return (style, prefix_only).
///
/// When `prefix_only` is true, only the first character should be styled.
fn classify_diff_line(line: &str) -> (Style, bool) {
    if line.starts_with("+++") || line.starts_with("---") {
        (
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            false,
        )
    } else if line.starts_with("@@") {
        (
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            false,
        )
    } else if line.starts_with('+') {
        (Style::default().fg(Color::Green), true)
    } else if line.starts_with('-') {
        (Style::default().fg(Color::Red), true)
    } else {
        (Style::default().fg(Color::DarkGray), false)
    }
}

/// Format a "no changes" message for identical files.
pub fn render_no_changes(filename: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        format!("No changes detected in {filename}"),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addition_lines_are_green() {
        let lines = render_diff("+new line");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.style.fg, Some(Color::Green));
    }

    #[test]
    fn deletion_lines_are_red() {
        let lines = render_diff("-old line");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.style.fg, Some(Color::Red));
    }

    #[test]
    fn hunk_headers_are_cyan_bold() {
        let lines = render_diff("@@ -1,3 +1,4 @@");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.style.fg, Some(Color::Cyan));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn file_headers_are_blue_bold() {
        let lines = render_diff("--- a/file.rs\n+++ b/file.rs");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Blue));
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn context_lines_are_gray() {
        let lines = render_diff(" context line");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn no_changes_message() {
        let lines = render_no_changes("test.rs");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains("No changes"));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn full_diff_rendering() {
        let diff = "--- a/main.rs\n+++ b/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n+    println!(\"extra\");\n }";
        let lines = render_diff(diff);
        assert!(lines.len() >= 7, "expected at least 7 lines, got {}", lines.len());
        // File headers (blue)
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Blue));
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Blue));
        // Hunk header (cyan)
        assert_eq!(lines[2].spans[0].style.fg, Some(Color::Cyan));
        // Context (gray)
        assert_eq!(lines[3].spans[0].style.fg, Some(Color::DarkGray));
        // Deletion (red prefix)
        assert_eq!(lines[4].spans[0].style.fg, Some(Color::Red));
        // Addition (green prefix)
        assert_eq!(lines[5].spans[0].style.fg, Some(Color::Green));
        assert_eq!(lines[6].spans[0].style.fg, Some(Color::Green));
    }
}
