//! TUI diff rendering with color highlighting.
//!
//! Renders unified diff output into styled ratatui Lines with:
//! - Green `+` lines for additions
//! - Red `-` lines for deletions
//! - Gray context lines
//! - Cyan `@@` hunk headers
//!
//! REQ-TUI-MOD-008 keyboard navigation:
//! - `n` — next hunk
//! - `p` — previous hunk
//! - `w` — toggle whitespace visibility
//! - `s` — toggle side-by-side / unified layout
//! - `/` — search within diff

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Layout mode for diff display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Unified,
    SideBySide,
}

/// A single line within a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    context(String),
    deletion(String),
    addition(String),
}

impl DiffLine {
    fn as_str(&self) -> &str {
        match self {
            DiffLine::context(s) => s,
            DiffLine::deletion(s) => s,
            DiffLine::addition(s) => s,
        }
    }

    fn style(&self) -> Style {
        match self {
            DiffLine::context(_) => Style::default().fg(Color::DarkGray),
            DiffLine::deletion(_) => Style::default().fg(Color::Red),
            DiffLine::addition(_) => Style::default().fg(Color::Green),
        }
    }

    fn prefix(&self) -> char {
        match self {
            DiffLine::context(_) => ' ',
            DiffLine::deletion(_) => '-',
            DiffLine::addition(_) => '+',
        }
    }
}

/// A hunk within a diff, with start_line and the lines it contains.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub start_line: u32,
    pub lines: Vec<DiffLine>,
}

/// Mutable diff view state — hunk index, toggles, search.
#[derive(Debug, Clone)]
pub struct DiffState {
    pub hunks: Vec<Hunk>,
    pub current_hunk: usize,
    pub show_whitespace: bool,
    pub layout_mode: LayoutMode,
    pub search_active: bool,
    pub search_query: String,
}

/// Interactive diff view with keyboard navigation state.
#[derive(Debug)]
pub struct DiffView {
    pub state: DiffState,
}

impl DiffView {
    /// Create a new DiffView with the given state.
    pub fn new(state: DiffState) -> Self {
        Self { state }
    }

    /// Handle a keyboard input character and update state accordingly.
    /// Returns Some(()) if the key was handled, None if unrecognized.
    pub fn handle_key(&mut self, ch: char) -> Option<()> {
        match ch {
            'n' => {
                if self.state.current_hunk + 1 < self.state.hunks.len() {
                    self.state.current_hunk += 1;
                }
                Some(())
            }
            'p' => {
                if self.state.current_hunk > 0 {
                    self.state.current_hunk -= 1;
                }
                Some(())
            }
            'w' => {
                self.state.show_whitespace = !self.state.show_whitespace;
                Some(())
            }
            's' => {
                self.state.layout_mode = match self.state.layout_mode {
                    LayoutMode::Unified => LayoutMode::SideBySide,
                    LayoutMode::SideBySide => LayoutMode::Unified,
                };
                Some(())
            }
            '/' => {
                self.state.search_active = true;
                self.state.search_query.clear();
                Some(())
            }
            '\x1B' => {
                // Escape — deactivate search
                self.state.search_active = false;
                self.state.search_query.clear();
                Some(())
            }
            c if self.state.search_active => {
                self.state.search_query.push(c);
                Some(())
            }
            _ => None,
        }
    }
}

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

/// Render the current hunk of a DiffView into ratatui Lines.
/// Respects layout_mode and show_whitespace settings.
pub fn render_current_hunk(view: &DiffView) -> Vec<Line<'static>> {
    if view.state.hunks.is_empty() {
        return vec![Line::from(Span::styled(
            "No hunks in diff",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let hunk = &view.state.hunks[view.state.current_hunk];

    match view.state.layout_mode {
        LayoutMode::Unified => render_hunk_unified(hunk, view.state.show_whitespace),
        LayoutMode::SideBySide => render_hunk_side_by_side(hunk, view.state.show_whitespace),
    }
}

fn render_hunk_unified(hunk: &Hunk, show_whitespace: bool) -> Vec<Line<'static>> {
    hunk.lines
        .iter()
        .filter(|line| {
            if show_whitespace {
                true
            } else {
                !line.as_str().contains(' ')
            }
        })
        .map(|line| {
            let prefix = line.prefix().to_string();
            let styled_prefix = Span::styled(prefix, line.style());
            let content = if show_whitespace {
                line.as_str().to_string()
            } else {
                line.as_str().trim().to_string()
            };
            let display = if content.is_empty() { " " } else { &content };
            Line::from(vec![styled_prefix, Span::raw(display.to_string())])
        })
        .collect()
}

fn render_hunk_side_by_side(hunk: &Hunk, show_whitespace: bool) -> Vec<Line<'static>> {
    // Pair deletions and additions side-by-side
    let mut lines = Vec::new();
    let mut del_lines: Vec<Span<'static>> = Vec::new();
    let mut add_lines: Vec<Span<'static>> = Vec::new();

    for line in &hunk.lines {
        let content = if show_whitespace {
            line.as_str().to_string()
        } else {
            line.as_str().trim().to_string()
        };
        let display = if content.is_empty() { " " } else { &content };
        match line {
            DiffLine::deletion(_) => {
                del_lines.push(Span::raw(display.to_string()));
            }
            DiffLine::addition(_) => {
                add_lines.push(Span::raw(display.to_string()));
            }
            DiffLine::context(_) => {
                lines.push(Line::from(vec![
                    Span::styled(" ".to_string(), Style::default().fg(Color::DarkGray)),
                    Span::raw(display.to_string()),
                ]));
            }
        }
    }

    // Interleave deletions and additions
    let max_len = del_lines.len().max(add_lines.len());
    for i in 0..max_len {
        let left = del_lines
            .get(i)
            .cloned()
            .unwrap_or_else(|| Span::raw(" ".to_string()));
        let right = add_lines
            .get(i)
            .cloned()
            .unwrap_or_else(|| Span::raw(" ".to_string()));
        lines.push(Line::from(vec![
            Span::styled("-".to_string(), Style::default().fg(Color::Red)),
            left,
            Span::raw("  ".to_string()),
            Span::styled("+".to_string(), Style::default().fg(Color::Green)),
            right,
        ]));
    }

    if lines.is_empty() {
        vec![Line::from(Span::styled(
            "(empty hunk)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        lines
    }
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
        assert!(
            lines.len() >= 7,
            "expected at least 7 lines, got {}",
            lines.len()
        );
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

    #[test]
    fn diff_view_n_key_advances_hunk() {
        let state = DiffState {
            hunks: vec![
                Hunk {
                    start_line: 1,
                    lines: vec![DiffLine::context("line1".into())],
                },
                Hunk {
                    start_line: 10,
                    lines: vec![DiffLine::context("line2".into())],
                },
            ],
            current_hunk: 0,
            show_whitespace: false,
            layout_mode: LayoutMode::Unified,
            search_active: false,
            search_query: String::new(),
        };
        let mut view = DiffView::new(state);
        view.handle_key('n');
        assert_eq!(view.state.current_hunk, 1);
    }

    #[test]
    fn diff_view_n_at_last_hunk_stays() {
        let state = DiffState {
            hunks: vec![
                Hunk {
                    start_line: 1,
                    lines: vec![DiffLine::context("line1".into())],
                },
                Hunk {
                    start_line: 10,
                    lines: vec![DiffLine::context("line2".into())],
                },
            ],
            current_hunk: 1,
            show_whitespace: false,
            layout_mode: LayoutMode::Unified,
            search_active: false,
            search_query: String::new(),
        };
        let mut view = DiffView::new(state);
        view.handle_key('n');
        assert_eq!(view.state.current_hunk, 1);
    }

    #[test]
    fn diff_view_p_at_first_hunk_stays() {
        let state = DiffState {
            hunks: vec![
                Hunk {
                    start_line: 1,
                    lines: vec![DiffLine::context("line1".into())],
                },
                Hunk {
                    start_line: 10,
                    lines: vec![DiffLine::context("line2".into())],
                },
            ],
            current_hunk: 0,
            show_whitespace: false,
            layout_mode: LayoutMode::Unified,
            search_active: false,
            search_query: String::new(),
        };
        let mut view = DiffView::new(state);
        view.handle_key('p');
        assert_eq!(view.state.current_hunk, 0);
    }

    #[test]
    fn diff_view_slash_activates_search() {
        let mut view = DiffView::new(new_test_state());
        view.handle_key('/');
        assert!(view.state.search_active);
    }

    #[test]
    fn diff_view_escape_deactivates_search() {
        let mut view = DiffView::new(DiffState {
            hunks: vec![],
            current_hunk: 0,
            show_whitespace: false,
            layout_mode: LayoutMode::Unified,
            search_active: true,
            search_query: "foo".into(),
        });
        view.handle_key('\x1B');
        assert!(!view.state.search_active);
        assert!(view.state.search_query.is_empty());
    }

    fn new_test_state() -> DiffState {
        DiffState {
            hunks: vec![],
            current_hunk: 0,
            show_whitespace: false,
            layout_mode: LayoutMode::Unified,
            search_active: false,
            search_query: String::new(),
        }
    }
}
