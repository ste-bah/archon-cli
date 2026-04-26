//! TASK-#208 SLASH-SEARCH /search slash-command handler.
//!
//! `/search <query>` performs a recursive case-insensitive substring
//! match on file basenames within `working_dir`, capping the result
//! list at `MAX_RESULTS` (200) to keep the overlay scroll bounded.
//! Matches are emitted as a `TuiEvent::ShowSearchResults { query,
//! entries }` and the TUI overlay (TASK-#208 search_results screen)
//! highlights the matched substring inline.
//!
//! # Architecture (overlay command)
//!
//! Mirrors TUI-627 `/skills` and TASK-#207 `/files` exactly:
//!
//!   - Reuses the layer-0 `archon_tui::app::FileEntry` DTO (originally
//!     introduced for #207).
//!   - New `TuiEvent::ShowSearchResults { query, entries }` variant
//!     (in BOTH `events.rs` and `app.rs` per the dual-enum structural
//!     debt — see TASK-#207 commit body for the consolidation
//!     follow-up).
//!   - New `SearchResults` screen at
//!     `crates/archon-tui/src/screens/search_results.rs` with
//!     selected_index + select_next/prev nav + render with
//!     case-insensitive substring highlighting.
//!   - `App::search_results: Option<SearchResults>` field.
//!   - Event-loop arm constructs `SearchResults::new(query, entries)`.
//!   - Input priority branch: Up/Down nav, Enter injects
//!     `@<absolute-path> ` and closes, Esc closes without injection.
//!     No descend semantics (results span many directories — flat
//!     list).
//!   - `Searcher` trait seam — production `RealSearcher` walks the
//!     directory tree via `walkdir::WalkDir`; tests inject
//!     `MockSearcher`.
//!
//! # Filtering and limits
//!
//! - Recursive walk via `walkdir::WalkDir` with `max_depth(MAX_DEPTH=8)`.
//! - Same `SKIP_DIRS` hardcoded list as `/files` (TASK-#207's
//!   `walker::SKIP_DIRS`): `.git`, `.hg`, `.svn`, `target`,
//!   `node_modules`, `dist`, `build`, `.cache`, `.venv`, `__pycache__`.
//!   Dotfiles + dot-directories filtered.
//! - Substring match is case-insensitive on the FILE BASENAME only
//!   (not full path), to match user intent ("/search foo" should find
//!   `src/foo.rs`, not every file under a parent dir whose path
//!   contains `foo`). Directories are NOT included — the picker is
//!   for files only.
//! - Cap at `MAX_RESULTS=200` matches. Walk stops early once cap
//!   reached.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_tui::app::{FileEntry, TuiEvent};

use crate::command::registry::{CommandContext, CommandHandler};

const MAX_RESULTS: usize = 200;
const MAX_DEPTH: usize = 8;

/// Hardcoded skip-list — same as the `/files` walker. Directories
/// matching these basenames are NOT descended into during the
/// recursive walk.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "dist",
    "build",
    ".cache",
    ".venv",
    "__pycache__",
];

/// Seam — tests inject `MockSearcher`, production uses
/// `RealSearcher` which delegates to `walkdir::WalkDir`.
pub(crate) trait Searcher: Send + Sync {
    fn search(&self, root: &Path, query: &str) -> Vec<FileEntry>;
}

/// Default `Searcher` impl — `walkdir::WalkDir`-based recursive
/// search.
pub(crate) struct RealSearcher;

impl Searcher for RealSearcher {
    fn search(&self, root: &Path, query: &str) -> Vec<FileEntry> {
        let query_lc = query.to_lowercase();
        let mut out: Vec<FileEntry> = Vec::with_capacity(64);

        let walker = walkdir::WalkDir::new(root)
            .max_depth(MAX_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip hardcoded build/VCS dirs + dotfiles+dotdirs.
                let name_os = e.file_name();
                let name = match name_os.to_str() {
                    Some(s) => s,
                    None => return false,
                };
                if e.depth() == 0 {
                    return true; // Don't filter the root itself
                }
                if name.starts_with('.') {
                    return false;
                }
                if SKIP_DIRS.iter().any(|d| *d == name) {
                    return false;
                }
                true
            });

        for entry in walker.flatten() {
            if out.len() >= MAX_RESULTS {
                break;
            }
            // We only care about regular files here — directories
            // pass `filter_entry` so the walk descends, but they
            // shouldn't appear in the results.
            let file_type = entry.file_type();
            if !file_type.is_file() {
                continue;
            }
            let basename = match entry.file_name().to_str() {
                Some(s) => s,
                None => continue,
            };
            if !basename.to_lowercase().contains(&query_lc) {
                continue;
            }
            out.push(FileEntry {
                name: basename.to_string(),
                path: entry.path().to_path_buf(),
                is_dir: false,
            });
        }
        out
    }
}

/// `/search` handler.
pub(crate) struct SearchHandler {
    searcher: Arc<dyn Searcher>,
}

impl SearchHandler {
    pub(crate) fn new() -> Self {
        Self {
            searcher: Arc::new(RealSearcher),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_searcher(searcher: Arc<dyn Searcher>) -> Self {
        Self { searcher }
    }
}

impl CommandHandler for SearchHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let query = args
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        if query.is_empty() {
            return Err(anyhow::anyhow!(
                "/search requires a query — try `/search <substring>`"
            ));
        }

        let working_dir: PathBuf = ctx
            .working_dir
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "SearchHandler invoked without working_dir populated \
                     — build_command_context bug"
                )
            })?
            .clone();

        let entries = self.searcher.search(&working_dir, &query);

        ctx.emit(TuiEvent::ShowSearchResults { query, entries });
        Ok(())
    }

    fn description(&self) -> &str {
        "Search files in the working directory by basename substring"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    struct MockSearcher {
        result: std::sync::Mutex<Option<Vec<FileEntry>>>,
        captured_query: std::sync::Mutex<Option<String>>,
        captured_root: std::sync::Mutex<Option<PathBuf>>,
    }

    impl MockSearcher {
        fn returning(entries: Vec<FileEntry>) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(entries)),
                captured_query: std::sync::Mutex::new(None),
                captured_root: std::sync::Mutex::new(None),
            }
        }
    }

    impl Searcher for MockSearcher {
        fn search(&self, root: &Path, query: &str) -> Vec<FileEntry> {
            *self.captured_root.lock().unwrap() = Some(root.to_path_buf());
            *self.captured_query.lock().unwrap() = Some(query.to_string());
            self.result.lock().unwrap().take().unwrap_or_default()
        }
    }

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            is_dir: false,
        }
    }

    #[test]
    fn empty_query_returns_err() {
        let handler = SearchHandler::with_searcher(Arc::new(MockSearcher::returning(vec![])));
        let (mut ctx, _rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp"))
            .build();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("requires a query") || msg.contains("query"),
            "expected query-required error; got: {}",
            msg
        );
    }

    #[test]
    fn whitespace_only_query_returns_err() {
        let handler = SearchHandler::with_searcher(Arc::new(MockSearcher::returning(vec![])));
        let (mut ctx, _rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp"))
            .build();
        let result = handler.execute(&mut ctx, &[String::from("   ")]);
        assert!(result.is_err());
    }

    #[test]
    fn execute_without_working_dir_returns_err() {
        let handler = SearchHandler::with_searcher(Arc::new(MockSearcher::returning(vec![])));
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[String::from("foo")]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(msg.contains("working_dir"));
    }

    #[test]
    fn execute_emits_show_search_results_with_query_and_entries() {
        let entries = vec![entry("foo.rs"), entry("foobar.txt")];
        let mock = Arc::new(MockSearcher::returning(entries.clone()));
        let handler = SearchHandler::with_searcher(mock.clone());
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp/proj"))
            .build();
        handler.execute(&mut ctx, &[String::from("foo")]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowSearchResults { query, entries: e } => {
                assert_eq!(query, "foo");
                assert_eq!(e.len(), 2);
                assert_eq!(e[0].name, "foo.rs");
            }
            other => panic!("expected ShowSearchResults, got {:?}", other),
        }
        // Verify the searcher saw the right inputs.
        assert_eq!(
            mock.captured_query.lock().unwrap().clone().unwrap(),
            "foo"
        );
        assert_eq!(
            mock.captured_root.lock().unwrap().clone().unwrap(),
            PathBuf::from("/tmp/proj")
        );
    }

    #[test]
    fn execute_joins_multi_word_query() {
        let mock = Arc::new(MockSearcher::returning(vec![]));
        let handler = SearchHandler::with_searcher(mock.clone());
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp"))
            .build();
        handler
            .execute(
                &mut ctx,
                &[
                    String::from("foo"),
                    String::from("bar"),
                    String::from("baz"),
                ],
            )
            .unwrap();
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::ShowSearchResults { query, .. } => {
                assert_eq!(query, "foo bar baz");
            }
            other => panic!("expected ShowSearchResults, got {:?}", other),
        }
        assert_eq!(
            mock.captured_query.lock().unwrap().clone().unwrap(),
            "foo bar baz"
        );
    }

    #[test]
    fn execute_with_empty_results_still_emits_event() {
        let handler = SearchHandler::with_searcher(Arc::new(MockSearcher::returning(vec![])));
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp"))
            .build();
        handler.execute(&mut ctx, &[String::from("nomatch")]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowSearchResults { query, entries } => {
                assert_eq!(query, "nomatch");
                assert!(entries.is_empty());
            }
            other => panic!("expected ShowSearchResults, got {:?}", other),
        }
    }

    #[test]
    fn description_and_aliases() {
        let h = SearchHandler::new();
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    fn real_searcher_finds_basename_match_in_tempdir() {
        // End-to-end test of `RealSearcher` against a real tempdir
        // (no MockSearcher). Walks one level to verify the
        // walkdir-based pipeline is wired correctly. Larger smoke
        // happens in the Gate-5 ignored test below.
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("search_real_{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("alpha-foo.rs"), b"x").unwrap();
        std::fs::write(root.join("beta.rs"), b"y").unwrap();
        std::fs::write(root.join("gamma-foobar.txt"), b"z").unwrap();
        let real = RealSearcher;
        let results = real.search(&root, "foo");
        assert_eq!(results.len(), 2, "expected 2 matches; got {:?}", results);
        let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"alpha-foo.rs"));
        assert!(names.contains(&"gamma-foobar.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn real_searcher_skips_hidden_dirs_and_dotfiles() {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("search_skip_{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root.join(".git")).unwrap();
        std::fs::create_dir_all(&root.join("target")).unwrap();
        std::fs::write(root.join(".git").join("foo.rs"), b"x").unwrap();
        std::fs::write(root.join("target").join("foo.rs"), b"x").unwrap();
        std::fs::write(root.join(".hidden-foo"), b"x").unwrap();
        std::fs::write(root.join("visible-foo.rs"), b"x").unwrap();
        let real = RealSearcher;
        let results = real.search(&root, "foo");
        let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
        // Only visible-foo.rs should match — .git/foo.rs, target/foo.rs,
        // and .hidden-foo should all be filtered.
        assert_eq!(results.len(), 1, "expected 1 match; got {:?}", names);
        assert_eq!(results[0].name, "visible-foo.rs");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn real_searcher_caps_at_max_results() {
        // Sanity: if a directory has more than 200 matches, the
        // walker should stop at 200 (MAX_RESULTS). Verified with a
        // directory of 250 files all containing "match".
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("search_cap_{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..250 {
            std::fs::write(root.join(format!("match-{i:03}.rs")), b"x").unwrap();
        }
        let real = RealSearcher;
        let results = real.search(&root, "match");
        assert_eq!(results.len(), MAX_RESULTS, "expected exactly MAX_RESULTS");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn search_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("search") must return Some;
        // dispatch with arg "smoke" against a real tempdir must emit
        // a ShowSearchResults event with query="smoke".
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("search")
            .expect("search must be registered in default_registry()");

        let pid = std::process::id();
        let tmp = std::env::temp_dir().join(format!("search_smoke_{pid}"));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("smoke-test.rs"), b"x").unwrap();
        let (mut ctx, mut rx) = CtxBuilder::new().with_working_dir(tmp.clone()).build();
        let result = handler.execute(&mut ctx, &[String::from("smoke")]);
        let _ = std::fs::remove_dir_all(&tmp);

        result.unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowSearchResults { query, entries } => {
                assert_eq!(query, "smoke");
                assert!(entries.iter().any(|e| e.name == "smoke-test.rs"));
            }
            other => panic!("expected ShowSearchResults, got {:?}", other),
        }
    }
}
