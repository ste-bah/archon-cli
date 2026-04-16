//! Memory file selector screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// MemoryStore is the abstract interface for loading memory entries.
/// Implementors can fetch from filesystem, network, or in-memory store.
pub trait MemoryStore: Send + Sync {
    /// Load all memory entries, returning path, size_kb, and modified timestamp.
    fn load_entries(&self) -> Vec<MemoryEntry>;

    /// Get entries filtered by query string (substring match on path).
    fn filter_entries(&self, entries: &[MemoryEntry], query: &str) -> Vec<MemoryEntry> {
        if query.is_empty() {
            entries.to_vec()
        } else {
            let q = query.to_lowercase();
            entries.iter()
                .filter(|e| e.path.to_lowercase().contains(&q))
                .cloned()
                .collect()
        }
    }
}

/// A memory entry in the browser.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub path: String,
    pub size_kb: u32,
    pub modified: String,
}

/// Memory browser with virtualized list and query filter.
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
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&MemoryEntry> {
        self.list.selected()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query string and filter.
    pub fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
        self.rebuild_filtered();
    }

    /// Set the full entries list and reset filter.
    pub fn set_entries(&mut self, entries: Vec<MemoryEntry>) {
        self.entries = entries;
        self.query.clear();
        self.rebuild_filtered();
    }

    fn rebuild_filtered(&mut self) {
        let filtered: Vec<MemoryEntry> = if self.query.is_empty() {
            self.entries.clone()
        } else {
            let q = self.query.to_lowercase();
            self.entries.iter()
                .filter(|e| e.path.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
        self.list.set_items(filtered);
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn page_up(&mut self) {
        self.list.page_up();
    }

    pub fn page_down(&mut self) {
        self.list.page_down();
    }

    /// Render memory browser into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Memory Files{}{}",
                if self.query.is_empty() { "" } else { " — " },
                if self.query.is_empty() { "" } else { &self.query }
            ));

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|e| {
            ListItem::new(format!("{} ({} KB)", e.path, e.size_kb))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for MemoryBrowser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_browser_empty() {
        let browser = MemoryBrowser::new();
        assert!(browser.is_empty());
        assert_eq!(browser.query(), "");
    }

    #[test]
    fn set_entries_updates_list() {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![
            MemoryEntry { path: "/a/b.txt".into(), size_kb: 10, modified: "2m ago".into() },
            MemoryEntry { path: "/c/d.txt".into(), size_kb: 20, modified: "5m ago".into() },
        ]);
        assert_eq!(browser.len(), 2);
    }

    #[test]
    fn set_query_filters_list() {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![
            MemoryEntry { path: "/foo/bar.txt".into(), size_kb: 10, modified: "1m".into() },
            MemoryEntry { path: "/baz/bar.txt".into(), size_kb: 20, modified: "2m".into() },
            MemoryEntry { path: "/foo/baz.txt".into(), size_kb: 30, modified: "3m".into() },
        ]);
        browser.set_query("foo");
        assert_eq!(browser.len(), 2); // two entries with "foo" in path
        browser.set_query("bar");
        assert_eq!(browser.len(), 2); // /foo/bar.txt and /baz/bar.txt
    }

    #[test]
    fn cursor_wraps() {
        let mut browser = MemoryBrowser::new();
        browser.set_entries(vec![
            MemoryEntry { path: "/a".into(), size_kb: 1, modified: "1m".into() },
            MemoryEntry { path: "/b".into(), size_kb: 2, modified: "2m".into() },
        ]);
        browser.move_down();
        assert_eq!(browser.selected_index(), 1);
        browser.move_down();
        assert_eq!(browser.selected_index(), 0); // wrap
    }
}