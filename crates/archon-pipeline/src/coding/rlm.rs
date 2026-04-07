//! RLM (Run-Level Memory) Context Store.
//!
//! Holds agent outputs within a single pipeline run as a `HashMap<String, String>`
//! keyed by namespace (e.g. `coding/understanding/task-analysis`).
//! Supports JSON persistence and LEANN fallback on missing namespaces.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// LeannSearcher trait (injectable for testing)
// ---------------------------------------------------------------------------

/// Trait for LEANN semantic search fallback when a namespace is missing.
pub trait LeannSearcher: Send + Sync {
    /// Search LEANN index for content matching the given query.
    fn search(&self, query: &str) -> String;
}

// ---------------------------------------------------------------------------
// RlmStore
// ---------------------------------------------------------------------------

/// Run-level memory context store.
///
/// Stores agent outputs keyed by namespace strings. Provides JSON round-trip
/// persistence and optional LEANN fallback for missing namespaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmStore {
    version: u32,
    namespaces: HashMap<String, String>,
}

impl RlmStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            version: 1,
            namespaces: HashMap::new(),
        }
    }

    /// Write content to a namespace, overwriting any previous value.
    pub fn write(&mut self, namespace: &str, content: &str) {
        self.namespaces
            .insert(namespace.to_string(), content.to_string());
    }

    /// Read content from a namespace, returning `None` if not present.
    pub fn read(&self, namespace: &str) -> Option<String> {
        self.namespaces.get(namespace).cloned()
    }

    /// Read content from a namespace, falling back to LEANN search if missing.
    pub fn read_or_search(&self, namespace: &str, searcher: &dyn LeannSearcher) -> String {
        match self.namespaces.get(namespace) {
            Some(content) => content.clone(),
            None => searcher.search(namespace),
        }
    }

    /// Persist the store to a JSON file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a store from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

    /// Return all stored namespace keys.
    pub fn namespaces(&self) -> Vec<&str> {
        self.namespaces.keys().map(|k| k.as_str()).collect()
    }

    /// Remove all entries from the store.
    pub fn clear(&mut self) {
        self.namespaces.clear();
    }
}

impl Default for RlmStore {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Mock LeannSearcher
    // -----------------------------------------------------------------------

    /// Mock searcher that records calls and returns a canned response.
    struct MockSearcher {
        response: String,
        call_count: AtomicUsize,
    }

    impl MockSearcher {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl LeannSearcher for MockSearcher {
        fn search(&self, _query: &str) -> String {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.response.clone()
        }
    }

    // -----------------------------------------------------------------------
    // 1. write then read returns stored content
    // -----------------------------------------------------------------------

    #[test]
    fn write_then_read_returns_stored_content() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "analyzed the task");

        assert_eq!(
            store.read("coding/understanding/task-analysis"),
            Some("analyzed the task".to_string()),
        );
    }

    #[test]
    fn write_overwrites_previous_value() {
        let mut store = RlmStore::new();
        store.write("coding/design/api-design", "v1");
        store.write("coding/design/api-design", "v2");

        assert_eq!(
            store.read("coding/design/api-design"),
            Some("v2".to_string()),
        );
    }

    // -----------------------------------------------------------------------
    // 2. read on missing namespace returns None
    // -----------------------------------------------------------------------

    #[test]
    fn read_missing_namespace_returns_none() {
        let store = RlmStore::new();
        assert_eq!(store.read("coding/understanding/nonexistent"), None);
    }

    #[test]
    fn read_missing_after_clear_returns_none() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "data");
        store.clear();

        assert_eq!(store.read("coding/understanding/task-analysis"), None);
    }

    // -----------------------------------------------------------------------
    // 3. read_or_search triggers LEANN fallback on missing namespace
    // -----------------------------------------------------------------------

    #[test]
    fn read_or_search_returns_stored_content_when_present() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "stored content");

        let searcher = MockSearcher::new("fallback content");
        let result = store.read_or_search("coding/understanding/task-analysis", &searcher);

        assert_eq!(result, "stored content");
        assert_eq!(searcher.calls(), 0, "searcher should not be called when namespace exists");
    }

    #[test]
    fn read_or_search_falls_back_to_leann_on_missing_namespace() {
        let store = RlmStore::new();
        let searcher = MockSearcher::new("leann fallback result");

        let result = store.read_or_search("coding/understanding/missing", &searcher);

        assert_eq!(result, "leann fallback result");
        assert_eq!(searcher.calls(), 1, "searcher should be called exactly once");
    }

    // -----------------------------------------------------------------------
    // 4. save produces valid JSON; load restores identical state
    // -----------------------------------------------------------------------

    #[test]
    fn save_produces_valid_json() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "content here");

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rlm.json");
        store.save(&path).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed["version"], 1);
        assert_eq!(
            parsed["namespaces"]["coding/understanding/task-analysis"],
            "content here",
        );
    }

    #[test]
    fn load_restores_identical_state() {
        let mut store = RlmStore::new();
        store.write("coding/design/api-design", "api spec");
        store.write("coding/implementation/parser", "parser code");

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rlm.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();
        assert_eq!(loaded.read("coding/design/api-design"), Some("api spec".to_string()));
        assert_eq!(loaded.read("coding/implementation/parser"), Some("parser code".to_string()));
    }

    // -----------------------------------------------------------------------
    // 5. JSON round-trip: save then load yields equal HashMap contents
    // -----------------------------------------------------------------------

    #[test]
    fn json_round_trip_preserves_all_entries() {
        let mut store = RlmStore::new();
        let entries = vec![
            ("coding/understanding/task-analysis", "task analysis output"),
            ("coding/understanding/codebase-scan", "scan results"),
            ("coding/design/api-design", "api design doc"),
            ("coding/implementation/core-impl", "implementation code"),
            ("coding/testing/unit-tests", "test suite"),
            ("coding/refinement/optimization", "perf notes"),
        ];

        for (ns, content) in &entries {
            store.write(ns, content);
        }

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("roundtrip.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();

        // Verify every entry survived the round-trip.
        for (ns, content) in &entries {
            assert_eq!(
                loaded.read(ns),
                Some(content.to_string()),
                "namespace {ns} should survive round-trip",
            );
        }

        // Verify no extra entries appeared.
        assert_eq!(loaded.namespaces().len(), entries.len());
    }

    #[test]
    fn round_trip_empty_store() {
        let store = RlmStore::new();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();
        assert!(loaded.namespaces().is_empty());
    }

    // -----------------------------------------------------------------------
    // 6. namespaces returns all stored keys
    // -----------------------------------------------------------------------

    #[test]
    fn namespaces_returns_all_keys() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "a");
        store.write("coding/design/api-design", "b");
        store.write("coding/testing/integration", "c");

        let mut ns = store.namespaces();
        ns.sort();

        assert_eq!(
            ns,
            vec![
                "coding/design/api-design",
                "coding/testing/integration",
                "coding/understanding/task-analysis",
            ],
        );
    }

    // -----------------------------------------------------------------------
    // 7. clear empties the store
    // -----------------------------------------------------------------------

    #[test]
    fn clear_empties_the_store() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/task-analysis", "data");
        store.write("coding/design/api-design", "data");

        assert_eq!(store.namespaces().len(), 2);

        store.clear();

        assert!(store.namespaces().is_empty());
        assert_eq!(store.read("coding/understanding/task-analysis"), None);
    }

    // -----------------------------------------------------------------------
    // 8. Namespace keys use coding/<phase>/<domain> convention
    // -----------------------------------------------------------------------

    #[test]
    fn namespace_convention_coding_phase_domain() {
        let mut store = RlmStore::new();

        // All six phases from the coding pipeline.
        let phase_namespaces = vec![
            "coding/understanding/task-analysis",
            "coding/understanding/codebase-scan",
            "coding/design/api-design",
            "coding/design/feasibility",
            "coding/wiring-plan/integration-arch",
            "coding/implementation/core-impl",
            "coding/testing/unit-tests",
            "coding/refinement/optimization",
        ];

        for ns in &phase_namespaces {
            store.write(ns, "content");
        }

        for ns in store.namespaces() {
            let parts: Vec<&str> = ns.split('/').collect();
            assert!(
                parts.len() >= 3,
                "namespace '{ns}' should have at least 3 segments: coding/<phase>/<domain>",
            );
            assert_eq!(
                parts[0], "coding",
                "namespace '{ns}' should start with 'coding'",
            );
        }
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn write_empty_content() {
        let mut store = RlmStore::new();
        store.write("coding/understanding/empty", "");

        assert_eq!(
            store.read("coding/understanding/empty"),
            Some(String::new()),
        );
    }

    #[test]
    fn write_large_content() {
        let mut store = RlmStore::new();
        let large = "x".repeat(100_000);
        store.write("coding/implementation/big", &large);

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("large.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();
        assert_eq!(loaded.read("coding/implementation/big").unwrap().len(), 100_000);
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = RlmStore::load(Path::new("/tmp/does_not_exist_rlm_test.json"));
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_json_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not valid json {{{").unwrap();

        let result = RlmStore::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn default_creates_empty_store() {
        let store = RlmStore::default();
        assert!(store.namespaces().is_empty());
        assert_eq!(store.read("anything"), None);
    }

    #[test]
    fn unicode_content_round_trips() {
        let mut store = RlmStore::new();
        let content = "日本語テスト 🦀 émojis ñ";
        store.write("coding/understanding/unicode", content);

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("unicode.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();
        assert_eq!(
            loaded.read("coding/understanding/unicode"),
            Some(content.to_string()),
        );
    }

    #[test]
    fn content_with_json_special_chars() {
        let mut store = RlmStore::new();
        let tricky = r#"{"key": "value", "escaped": "line\nnewline"}"#;
        store.write("coding/design/tricky", tricky);

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("special.json");
        store.save(&path).unwrap();

        let loaded = RlmStore::load(&path).unwrap();
        assert_eq!(
            loaded.read("coding/design/tricky"),
            Some(tricky.to_string()),
        );
    }
}
