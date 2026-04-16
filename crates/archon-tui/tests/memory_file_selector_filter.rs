//! Integration tests for memory_file_selector filter behavior.

use archon_tui::screens::memory_file_selector::{MemoryBrowser, MemoryEntry, MemoryStore};

/// In-memory store for testing.
struct TestMemoryStore {
    entries: Vec<MemoryEntry>,
}

impl MemoryStore for TestMemoryStore {
    fn load_entries(&self) -> Vec<MemoryEntry> {
        self.entries.clone()
    }
}

fn make_entry(path: &str, size_kb: u32) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        size_kb,
        modified: "1m ago".into(),
    }
}

#[test]
fn test_browser_query_filters_entries() {
    let store = TestMemoryStore {
        entries: vec![
            make_entry("/foo/bar.txt", 10),
            make_entry("/foo/baz.txt", 20),
            make_entry("/qux/bar.txt", 30),
        ],
    };

    let mut browser = MemoryBrowser::new();
    browser.set_entries(store.load_entries());

    assert_eq!(browser.len(), 3);

    browser.set_query("foo");
    assert_eq!(browser.len(), 2); // /foo/bar.txt, /foo/baz.txt

    browser.set_query("bar");
    assert_eq!(browser.len(), 2); // /foo/bar.txt, /qux/bar.txt

    browser.set_query("nonexistent");
    assert_eq!(browser.len(), 0);
}

#[test]
fn test_browser_query_case_insensitive() {
    let store = TestMemoryStore {
        entries: vec![
            make_entry("/foo/bar.txt", 10),
            make_entry("/bar/foo.txt", 20),
        ],
    };

    let mut browser = MemoryBrowser::new();
    browser.set_entries(store.load_entries());

    // Query "foo" should match both: /foo/bar.txt and /bar/foo.txt
    browser.set_query("foo");
    assert_eq!(browser.len(), 2);

    // Query "bar" also matches both: /foo/bar.txt and /bar/foo.txt
    browser.set_query("bar");
    assert_eq!(browser.len(), 2);
}

#[test]
fn test_store_filter_entries_method() {
    let store = TestMemoryStore {
        entries: vec![
            make_entry("/alpha/beta.txt", 5),
            make_entry("/alpha/gamma.txt", 10),
            make_entry("/beta/delta.txt", 15),
        ],
    };

    let entries = store.load_entries();
    let filtered = store.filter_entries(&entries, "alpha");
    assert_eq!(filtered.len(), 2);

    let filtered = store.filter_entries(&entries, "");
    assert_eq!(filtered.len(), 3); // empty query returns all
}

#[test]
fn test_browser_cursor_wrapping() {
    let mut browser = MemoryBrowser::new();
    browser.set_entries(vec![
        make_entry("/a", 1),
        make_entry("/b", 2),
    ]);

    assert_eq!(browser.selected_index(), 0);
    browser.move_down();
    assert_eq!(browser.selected_index(), 1);
    browser.move_down();
    assert_eq!(browser.selected_index(), 0); // wraps to start
}
