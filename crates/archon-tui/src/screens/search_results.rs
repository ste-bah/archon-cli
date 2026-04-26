//! TASK-#208 SLASH-SEARCH search-results overlay (screen module).
//!
//! Single-file overlay (no sub-module split — reasonable line
//! count, mostly straightforward render code). Mirrors
//! `screens/skills_menu.rs` and `screens/file_picker/render.rs`
//! shape:
//!
//!   - `SearchResults` struct holds the query + a `Vec<FileEntry>`
//!     of matched paths and a selected_index.
//!   - Centered overlay 9/10-wide with selected-row cyan+bold.
//!   - Render highlights the matched query substring inside each
//!     row's path text — the part of the path that matched is
//!     shown in **bold cyan** while the surrounding path text uses
//!     the theme's foreground color.
//!
//! Input routing in `event_loop/input.rs`: Up/Down navigate; Enter
//! injects `@<absolute-path> ` into the input buffer and closes the
//! overlay; Esc cancels without injection.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::events::FileEntry;
use crate::theme::Theme;

/// Search-results overlay state.
pub struct SearchResults {
    /// The original query the user supplied to `/search <query>`.
    /// Used for the case-insensitive highlight match in the
    /// rendered rows.
    pub query: String,
    /// The matched file paths (cap'd at the slash handler's
    /// `max_results` ceiling, default 200).
    pub entries: Vec<FileEntry>,
    /// Index into `entries` of the highlighted row.
    pub selected_index: usize,
}

impl SearchResults {
    pub fn new(query: String, entries: Vec<FileEntry>) -> Self {
        Self {
            query,
            entries,
            selected_index: 0,
        }
    }

    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.entries.len();
    }

    pub fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.entries.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected_index)
    }

    /// Render the search-results overlay inside `area`.
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
            vec![
                ListItem::new(format!(" no matches for `{}` ", self.query))
                    .style(Style::default().fg(theme.fg)),
            ]
        } else {
            self.entries[start..end]
                .iter()
                .enumerate()
                .map(|(offset, entry)| {
                    let idx = start + offset;
                    let row_style = if idx == self.selected_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    let path_str = entry.path.display().to_string();
                    let prefix = format!(" {idx:>3}  ");
                    let spans = build_highlighted_spans(
                        &prefix,
                        &path_str,
                        &self.query,
                        row_style,
                        theme,
                        idx == self.selected_index,
                    );
                    ListItem::new(Line::from(spans))
                })
                .collect()
        };

        let title = format!(
            " /search — {} match(es) for `{}` (Up/Down, Enter pick, Esc cancel) ",
            self.entries.len(),
            self.query
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

/// Build the styled `Vec<Span>` for one row: prefix (in row style),
/// then the path with the query substring highlighted.
///
/// Highlight rule: case-insensitive find the FIRST occurrence of
/// `query` in `path`. If found, split into (before, match, after)
/// where match is rendered in bold cyan (or bold on the
/// already-cyan-bg selected row, since cyan-on-cyan would be
/// invisible). If not found (paths can pass the slash-handler
/// filter via parent-dir match while basename doesn't contain the
/// query), the row renders without highlight.
fn build_highlighted_spans(
    prefix: &str,
    path: &str,
    query: &str,
    row_style: Style,
    _theme: &Theme,
    is_selected: bool,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::with_capacity(4);
    out.push(Span::styled(prefix.to_string(), row_style));

    if query.is_empty() {
        out.push(Span::styled(path.to_string(), row_style));
        return out;
    }

    // Case-insensitive match of `query` against `path`.
    let path_lc = path.to_lowercase();
    let query_lc = query.to_lowercase();
    let match_byte_idx = path_lc.find(&query_lc);

    let match_byte_idx = match match_byte_idx {
        Some(i) => i,
        None => {
            out.push(Span::styled(path.to_string(), row_style));
            return out;
        }
    };

    // The lowercased path has the same byte length as the original
    // path for ASCII; for Unicode the `to_lowercase()` byte length
    // can differ. To stay safe, recompute the match boundary by
    // walking the original path's char-byte indices.
    let (before, match_segment, after) = split_at_case_insensitive(path, query, match_byte_idx);

    if !before.is_empty() {
        out.push(Span::styled(before.to_string(), row_style));
    }
    let highlight_style = if is_selected {
        // On the selected row (cyan bg + black fg), bold the match
        // since changing color would either match the bg or look
        // jarring.
        row_style.add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };
    out.push(Span::styled(match_segment.to_string(), highlight_style));
    if !after.is_empty() {
        out.push(Span::styled(after.to_string(), row_style));
    }
    out
}

/// Char-aware split of `path` around the case-insensitive match of
/// `query` whose lowercased version starts at `lc_byte_idx` in
/// `path.to_lowercase()`. Returns `(before, match, after)` of the
/// ORIGINAL path (preserving original case in the highlighted
/// segment). Falls back to the no-highlight case if any boundary
/// computation fails.
fn split_at_case_insensitive<'a>(
    path: &'a str,
    query: &str,
    lc_byte_idx: usize,
) -> (&'a str, &'a str, &'a str) {
    // Walk the original path one char at a time and accumulate
    // lowercased bytes until we reach `lc_byte_idx`.
    let mut accum_lc_bytes: usize = 0;
    let mut start_byte_in_path: Option<usize> = None;
    for (orig_byte, ch) in path.char_indices() {
        if accum_lc_bytes == lc_byte_idx {
            start_byte_in_path = Some(orig_byte);
            break;
        }
        accum_lc_bytes += ch.to_lowercase().map(|c| c.len_utf8()).sum::<usize>();
    }
    // Edge case: match starts at end of string (shouldn't happen
    // for a real find but defensively handle).
    if start_byte_in_path.is_none() && accum_lc_bytes == lc_byte_idx {
        start_byte_in_path = Some(path.len());
    }
    let start = match start_byte_in_path {
        Some(i) => i,
        None => return (path, "", ""),
    };

    // Compute the char-by-char end boundary by stepping `query`
    // chars forward from `start`.
    let mut end = start;
    let mut q_chars = query.chars();
    let path_after = &path[start..];
    for ch in path_after.chars() {
        match q_chars.next() {
            Some(_qc) => {
                end += ch.len_utf8();
            }
            None => break,
        }
    }
    if end > path.len() {
        end = path.len();
    }
    (&path[..start], &path[start..end], &path[end..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            is_dir: false,
        }
    }

    #[test]
    fn new_starts_at_zero() {
        let r = SearchResults::new("foo".into(), vec![entry("foo.txt"), entry("foobar.rs")]);
        assert_eq!(r.selected_index, 0);
        assert_eq!(r.query, "foo");
    }

    #[test]
    fn select_next_wraps() {
        let mut r = SearchResults::new("x".into(), vec![entry("a"), entry("b")]);
        r.select_next();
        assert_eq!(r.selected_index, 1);
        r.select_next();
        assert_eq!(r.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut r = SearchResults::new("x".into(), vec![entry("a"), entry("b")]);
        r.select_prev();
        assert_eq!(r.selected_index, 1);
    }

    #[test]
    fn empty_results_noop() {
        let mut r = SearchResults::new("x".into(), vec![]);
        r.select_next();
        r.select_prev();
        assert_eq!(r.selected_index, 0);
        assert!(r.selected().is_none());
    }

    #[test]
    fn split_ascii_match_in_middle() {
        let (before, m, after) = split_at_case_insensitive(
            "src/foo/bar.rs",
            "foo",
            "src/foo/bar.rs".to_lowercase().find("foo").unwrap(),
        );
        assert_eq!(before, "src/");
        assert_eq!(m, "foo");
        assert_eq!(after, "/bar.rs");
    }

    #[test]
    fn split_case_insensitive_preserves_original_case() {
        // Path has "Foo", query is "foo" lowercase — the match
        // segment should preserve the original "Foo" case.
        let path = "src/Foo/bar.rs";
        let lc_idx = path.to_lowercase().find("foo").unwrap();
        let (before, m, after) = split_at_case_insensitive(path, "foo", lc_idx);
        assert_eq!(before, "src/");
        assert_eq!(m, "Foo");
        assert_eq!(after, "/bar.rs");
    }

    #[test]
    fn split_match_at_start() {
        let path = "foo.rs";
        let lc_idx = path.to_lowercase().find("foo").unwrap();
        let (before, m, after) = split_at_case_insensitive(path, "foo", lc_idx);
        assert_eq!(before, "");
        assert_eq!(m, "foo");
        assert_eq!(after, ".rs");
    }

    #[test]
    fn split_match_at_end() {
        let path = "src/main_foo";
        let lc_idx = path.to_lowercase().find("foo").unwrap();
        let (before, m, after) = split_at_case_insensitive(path, "foo", lc_idx);
        assert_eq!(before, "src/main_");
        assert_eq!(m, "foo");
        assert_eq!(after, "");
    }
}
