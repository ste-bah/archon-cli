//! The `/memory files` overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Lists the `ARCHON.md` / `CLAUDE.md` files this session is carrying, in the
//! order they are layered into the system prompt: global first, then each
//! ancestor directory, then the working directory, so a later file overrides an
//! earlier one. That order is the whole point of the list — the rendered text
//! goes into every request and nothing reported where any of it came from.
//!
//! It is not the memory graph. `/memory list|search|clear` operates on stored
//! memories in Cozo; these are instruction files on disk, and conflating the
//! two under one command would make both harder to reason about.
//!
//! Read-only. The restored screen had a `MemoryStore` trait with no
//! implementor; there is no in-TUI editor to hand a selection to, so it reads
//! and closes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One instruction file in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub path: String,
    pub size_bytes: u64,
    /// Where in the hierarchy it came from: `global`, `ancestor`, `project`.
    pub scope: String,
}

/// Read-only browser over the loaded instruction files, with a query filter.
#[derive(Debug)]
pub struct MemoryBrowser {
    entries: Vec<MemoryEntry>,
    list: VirtualList<MemoryEntry>,
    query: String,
}

impl MemoryBrowser {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
            query: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, MemoryEntry);

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.rebuild_filtered();
    }

    /// Replace the entries and clear any filter.
    pub fn set_entries(&mut self, entries: Vec<MemoryEntry>) {
        self.entries = entries;
        self.query.clear();
        self.rebuild_filtered();
    }

    fn rebuild_filtered(&mut self) {
        let filtered: Vec<MemoryEntry> = if self.query.is_empty() {
            self.entries.clone()
        } else {
            let needle = self.query.to_lowercase();
            self.entries
                .iter()
                .filter(|entry| entry.path.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
        self.list.set_items(filtered);
    }

    /// Draw the file list into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.query.is_empty() {
            " Memory files — type to filter · Esc close ".to_string()
        } else {
            format!(" Memory files — filter: {} · Esc close ", self.query)
        };

        if self.list.is_empty() {
            let body = if self.entries.is_empty() {
                "No ARCHON.md or CLAUDE.md is being loaded."
            } else {
                "No file matches that filter."
            };
            crate::overlay::message(f, area, &title, body, theme);
            return;
        }

        // rows + header + two border lines.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 3, &title, theme);

        let widths = [
            Constraint::Length(9),
            Constraint::Min(20),
            Constraint::Length(10),
        ];

        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|entry| {
                Row::new([
                    entry.scope.clone(),
                    entry.path.clone(),
                    format_size(entry.size_bytes),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        crate::overlay::render_table(
            f,
            region,
            block,
            Row::new(["Scope", "Path", "Size"]),
            rows,
            &widths,
            self.list.selected_index(),
            theme,
        );
    }
}

impl Default for MemoryBrowser {
    fn default() -> Self {
        Self::new()
    }
}

/// Human-readable size. Bytes below a kilobyte, because an instruction file of
/// 300 bytes reading `0 KB` looks like an empty one.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{} KB", bytes / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, scope: &str) -> MemoryEntry {
        MemoryEntry {
            path: path.into(),
            size_bytes: 2048,
            scope: scope.into(),
        }
    }

    #[test]
    fn a_new_browser_is_empty() {
        assert!(MemoryBrowser::new().is_empty());
    }

    #[test]
    fn the_filter_narrows_by_path() {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![
            entry("/home/me/.archon/ARCHON.md", "global"),
            entry("/work/repo/ARCHON.md", "project"),
        ]);
        browser.set_query("repo");
        assert_eq!(browser.len(), 1);
        assert_eq!(
            browser.selected().map(|e| e.path.as_str()),
            Some("/work/repo/ARCHON.md")
        );
    }

    #[test]
    fn setting_entries_clears_the_filter() {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![entry("/a/ARCHON.md", "project")]);
        browser.set_query("nothing-matches");
        assert_eq!(browser.len(), 0);
        browser.set_entries(vec![entry("/a/ARCHON.md", "project")]);
        assert_eq!(browser.query(), "");
        assert_eq!(browser.len(), 1);
    }

    /// A 300-byte instruction file is not an empty one.
    #[test]
    fn small_files_are_sized_in_bytes() {
        assert_eq!(format_size(300), "300 B");
        assert_eq!(format_size(2048), "2 KB");
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn browser() -> MemoryBrowser {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![
            MemoryEntry {
                path: "/home/me/.archon/ARCHON.md".into(),
                size_bytes: 4096,
                scope: "global".into(),
            },
            MemoryEntry {
                path: "/work/repo/ARCHON.md".into(),
                size_bytes: 300,
                scope: "project".into(),
            },
        ]);
        browser
    }

    fn draw(browser: &MemoryBrowser) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| browser.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw memory browser");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn style_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<ratatui::style::Style> {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        for y in 0..area.height {
            let line: String = (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(column) = line.find(needle) {
                return Some(buffer[(column as u16, y)].style());
            }
        }
        None
    }

    #[test]
    fn paths_scopes_and_sizes_are_drawn() {
        let rendered = text(&draw(&browser()));
        assert!(rendered.contains("global"), "{rendered}");
        assert!(rendered.contains("project"), "{rendered}");
        assert!(rendered.contains("300 B"), "{rendered}");
        assert!(rendered.contains("Scope"), "header missing: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut browser = browser();
        let first = draw(&browser);
        let one = style_of(&first, "global").expect("first row");
        let two = style_of(&first, "project").expect("second row");
        assert_ne!(one, two, "selection is invisible");

        browser.move_down();
        assert_ne!(
            one,
            style_of(&draw(&browser), "global").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    /// Two different empty states: nothing loaded, and nothing matching.
    #[test]
    fn the_two_empty_states_are_distinguished() {
        assert!(
            text(&draw(&MemoryBrowser::new()))
                .contains("No ARCHON.md or CLAUDE.md is being loaded")
        );

        let mut browser = browser();
        browser.set_query("zzz");
        assert!(text(&draw(&browser)).contains("No file matches that filter."));
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&browser());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
