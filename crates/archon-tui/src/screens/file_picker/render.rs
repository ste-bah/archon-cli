//! TASK-#207 SLASH-FILES — render layer for the file-picker overlay.
//!
//! Mirrors `screens/skills_menu.rs::render` and
//! `screens/message_selector.rs::render` — centered overlay ~9/10
//! wide, height clamped to the screen, scrolling visible-slice that
//! keeps the selected row on-screen, selected row in cyan+bold.
//!
//! Each row prefixed with `[D]` for directories and `[F]` for
//! files. The breadcrumb (relative to picker root) is in the title
//! bar so the user always knows where they are.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use super::FilePicker;
use crate::theme::Theme;

impl FilePicker {
    /// Render the file-picker overlay inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let overlay_width = (area.width * 9 / 10)
            .max(70)
            .min(area.width.saturating_sub(2));
        let overlay_height = (self.entries.len() as u16 + 3)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = (area.width.saturating_sub(overlay_width)) / 2;
        let y = (area.height.saturating_sub(overlay_height)) / 2;
        let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

        f.render_widget(Clear, overlay_area);

        // Visible slice — keep selected row on-screen when the list
        // is taller than the overlay. Two rows are reserved for
        // borders, so at most `overlay_height - 2` rows are
        // available for items.
        let body_rows = overlay_height.saturating_sub(2) as usize;
        let total = self.entries.len();
        let start = if total <= body_rows {
            0
        } else if self.selected_index >= body_rows {
            self.selected_index + 1 - body_rows
        } else {
            0
        };
        let end = (start + body_rows).min(total);

        let items: Vec<ListItem<'_>> = if self.entries.is_empty() {
            vec![ListItem::new(" (empty directory) ").style(Style::default().fg(theme.fg))]
        } else {
            self.entries[start..end]
                .iter()
                .enumerate()
                .map(|(offset, entry)| {
                    let idx = start + offset;
                    let style = if idx == self.selected_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    let badge = if entry.is_dir { "[D]" } else { "[F]" };
                    let name = truncate_chars(&entry.name, 64);
                    let line = format!(" {idx:>3}  {badge}  {name}");
                    ListItem::new(line).style(style)
                })
                .collect()
        };

        let crumb = self.breadcrumb();
        let title = format!(
            " /files — {crumb} (Up/Down, Enter dir-descend or file-pick, Backspace ascend, Esc cancel) "
        );
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(theme.accent)),
        );
        f.render_widget(list, overlay_area);
    }
}

/// Truncate `s` to at most `max` characters (char-boundary safe)
/// with a trailing ellipsis when clipped. Mirrors `skills_menu.rs::
/// truncate_preview`.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_short_unchanged() {
        assert_eq!(truncate_chars("hello", 10), "hello");
    }

    #[test]
    fn truncate_chars_long_clipped() {
        let s: String = (0..100).map(|_| 'x').collect();
        let out = truncate_chars(&s, 64);
        assert_eq!(out.chars().count(), 64);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_chars_unicode_safe() {
        let s: String = (0..30).map(|_| 'α').collect();
        let out = truncate_chars(&s, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    // ─────────────────────────────────────────────────────────────────
    // TASK-CI-PHASE4-REGRESSION-FIX Part 2: render-fn coverage tests.
    //
    // Drive `FilePicker::render` against a `TestBackend` to lift this
    // file's line coverage from 32.93% over the 80% TUI threshold.
    // Pattern mirrors `tests/render_coverage.rs::buffer_to_string`.
    // ─────────────────────────────────────────────────────────────────

    use crate::events::FileEntry;
    use crate::screens::file_picker::FilePicker;
    use crate::theme::intj_theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    /// Render a FilePicker overlay into a TestBackend and return the
    /// flattened buffer string.
    fn render_to_string(p: &FilePicker, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("TestBackend");
        let theme = intj_theme();
        terminal
            .draw(|f| p.render(f, f.area(), &theme))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let area = buffer.area;
        let mut s = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buffer[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/proj/{name}")),
            is_dir,
        }
    }

    #[test]
    fn render_empty_directory_shows_placeholder() {
        let picker = FilePicker::new(PathBuf::from("/proj"), vec![]);
        let body = render_to_string(&picker, 80, 24);
        assert!(
            body.contains("(empty directory)"),
            "empty branch must render `(empty directory)` placeholder; got:\n{body}"
        );
        // Title bar should reference /files and the breadcrumb (`.` at root).
        assert!(body.contains("/files"));
    }

    #[test]
    fn render_single_file_shows_F_badge() {
        let picker = FilePicker::new(PathBuf::from("/proj"), vec![entry("Cargo.toml", false)]);
        let body = render_to_string(&picker, 100, 24);
        assert!(body.contains("Cargo.toml"));
        assert!(
            body.contains("[F]"),
            "regular file must show [F] badge; got:\n{body}"
        );
    }

    #[test]
    fn render_dir_entry_shows_D_badge() {
        let picker = FilePicker::new(PathBuf::from("/proj"), vec![entry("src", true)]);
        let body = render_to_string(&picker, 100, 24);
        assert!(body.contains("src"));
        assert!(
            body.contains("[D]"),
            "directory must show [D] badge; got:\n{body}"
        );
    }

    #[test]
    fn render_mixed_entries_shows_both_badges() {
        let picker = FilePicker::new(
            PathBuf::from("/proj"),
            vec![
                entry("src", true),
                entry("tests", true),
                entry("Cargo.toml", false),
                entry("README.md", false),
            ],
        );
        let body = render_to_string(&picker, 120, 24);
        assert!(body.contains("[D]"));
        assert!(body.contains("[F]"));
        assert!(body.contains("src"));
        assert!(body.contains("tests"));
        assert!(body.contains("Cargo.toml"));
        assert!(body.contains("README.md"));
    }

    #[test]
    fn render_with_selection_at_middle() {
        let entries: Vec<FileEntry> = (0..5).map(|i| entry(&format!("f{i}.rs"), false)).collect();
        let mut picker = FilePicker::new(PathBuf::from("/proj"), entries);
        picker.selected_index = 2;
        let body = render_to_string(&picker, 100, 24);
        for i in 0..5 {
            assert!(body.contains(&format!("f{i}.rs")), "row {i} missing");
        }
    }

    #[test]
    fn render_scrolls_to_keep_selection_visible() {
        // 50 entries in a small overlay forces the visible-slice scroll
        // path. Selection near end must stay on-screen; early entries
        // must scroll off.
        let entries: Vec<FileEntry> = (0..50)
            .map(|i| entry(&format!("file{i:02}.rs"), false))
            .collect();
        let mut picker = FilePicker::new(PathBuf::from("/proj"), entries);
        picker.selected_index = 40;
        let body = render_to_string(&picker, 80, 14);
        assert!(
            body.contains("file40.rs"),
            "selected entry must be on-screen; got:\n{body}"
        );
        assert!(
            !body.contains("file00.rs"),
            "early entries must scroll off-screen; got:\n{body}"
        );
    }

    #[test]
    fn render_breadcrumb_reflects_subdir() {
        // After descending into a subdir, breadcrumb shows `./<rel>`
        // in the title bar. We can't actually descend without a real
        // filesystem (would call walker::read_dir_entries), so simulate
        // by constructing the picker state directly.
        let mut picker = FilePicker::new(PathBuf::from("/proj"), vec![entry("inner.rs", false)]);
        picker.current_dir = PathBuf::from("/proj/src/sub");
        let body = render_to_string(&picker, 120, 24);
        assert!(
            body.contains("./src/sub"),
            "breadcrumb must show relative subdir path; got:\n{body}"
        );
    }

    #[test]
    fn render_long_filename_truncated_with_ellipsis() {
        // Filenames > 64 chars get char-aware truncation with `…`.
        let long_name = "a".repeat(80);
        let picker = FilePicker::new(PathBuf::from("/proj"), vec![entry(&long_name, false)]);
        let body = render_to_string(&picker, 100, 24);
        assert!(
            body.contains('…'),
            "long filename must include ellipsis on truncation; got:\n{body}"
        );
    }

    #[test]
    fn render_small_terminal_does_not_panic() {
        let picker = FilePicker::new(PathBuf::from("/proj"), vec![entry("x.rs", false)]);
        let body = render_to_string(&picker, 30, 8);
        assert!(!body.is_empty());
    }
}
