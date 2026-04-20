//! TASK-AGS-817: /memory slash-command handler (Option C, DIRECT pattern,
//! THIRD Batch-3 body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:521-525` and the legacy match arm
//! at `src/command/slash.rs:342-345` (the pre-AGS-817 async
//! `handle_memory_command` free function lived in this same file and
//! has been REPLACED with the sync handler below).
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! `archon_memory::MemoryTrait` (defined at
//! `crates/archon-memory/src/access.rs:24`) is FULLY SYNC. All 12 trait
//! methods are plain `fn ... -> Result<_, MemoryError>` — zero `async
//! fn`, zero `.await` needed anywhere in the handler body. The trait
//! carries a `Send + Sync` bound so `Arc<dyn MemoryTrait>` is cheap to
//! clone (atomic refcount bump, ~8 bytes). Consequently:
//!
//! - NO `MemorySnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp` / `/context`).
//! - NO `CommandEffect` variant (`clear_all` mutates the graph but via
//!   a direct sync call; no write-back to a `Mutex`-guarded
//!   `SlashCommandContext` field).
//! - A new `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` field
//!   (8 -> 9) populated UNCONDITIONALLY by `build_command_context`,
//!   mirroring the AGS-815 `session_id` cross-cutting precedent. Any
//!   future handler that needs a memory handle inherits this field for
//!   free without proliferating per-command builder match arms.
//!
//! The sole side effect is `ctx.tui_tx.try_send(TuiEvent::…)` — sync
//! and legal inside `CommandHandler::execute`. Matches
//! AGS-810/812/815/816 DIRECT-pattern precedent.
//!
//! # Why UNCONDITIONAL populate (not per-dispatch match arm)?
//!
//! `Arc::clone(&Arc<dyn MemoryTrait>)` is a single atomic refcount
//! bump. Every dispatch pays ~8 bytes + one atomic op regardless of
//! whether the target handler reads `memory`. Peer precedent:
//! AGS-815 session_id UNCONDITIONAL populate (36-byte UUID String
//! clone per dispatch) was accepted as negligible. Field count
//! 8 -> 9 (refactor threshold is 10+; post-AGS-817 projection is 9
//! because Batch-3 tail /export and /theme are DIRECT no-new-field
//! per pre-analysis).
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted async
//! `handle_memory_command` body:
//! - `"" | "list"` empty -> `TextDelta("\nNo memories stored.\n")`
//! - `"" | "list"` non-empty -> `TextDelta(format!("\nRecent memories \
//!   ({len}):\n  [{short_id}] {title} ({mtype}, {date})\n..."))`
//! - `"" | "list"` err -> `Error(format!("Memory graph error: {e}"))`
//! - `"search"` no arg -> `Error("Usage: /memory search <query>")`
//! - `"search"` empty results -> `TextDelta(format!("\nNo memories \
//!   matching \"{arg}\".\n"))`
//! - `"search"` non-empty -> `TextDelta(format!("\nMemories matching \
//!   \"{arg}\" ({len}):\n  [{short_id}] {title} -- {snippet}\n..."))`
//! - `"search"` err -> `Error(format!("Memory search error: {e}"))`
//! - `"clear"` ok -> `TextDelta(format!("\nCleared {n} memories from \
//!   the graph.\n"))`
//! - `"clear"` err -> `Error(format!("Failed to clear memories: {e}"))`
//! - unknown sub -> `Error(format!("Unknown memory subcommand: \
//!   {other}. Use list, search, or clear."))`
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching every peer migrated
//! handler (AGS-806..816). `/memory` output is best-effort informational
//! UI — dropping a message under 16-cap channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! The `truncate_str` UTF-8 helper is preserved byte-for-byte from the
//! pre-migration module (char-boundary safe, 80-byte limit with "..."
//! suffix).
//!
//! # Aliases
//!
//! Shipped pre-AGS-817: `&["mem"]` (from `declare_handler!`). Drift-
//! reconcile shipped-wins: the `mem` alias is PRESERVED. Dropping it
//! would regress any operator workflow depending on `/mem list` /
//! `/mem search ...` working today through the stub's dispatcher path.
//! Matches AGS-813 shipped-wins precedent for alias-set preservation.
//!
//! # Args-path reconciliation
//!
//! Shipped body used `input.strip_prefix("/memory").trim()` followed by
//! `rest.split_once(' ')` to split into subcommand + single-string
//! argument. The registry parser tokenizes on whitespace, so `args` is
//! already a `Vec<String>` of individual tokens. To preserve the
//! shipped semantics where `/memory search hello world` forwards
//! `"hello world"` (not `"hello"` alone) to `recall_memories`, the
//! handler:
//!
//! 1. Reads `args.first()` as the subcommand.
//! 2. Joins `args.get(1..)` with a single space to rebuild the
//!    original single-string query argument.
//!
//! Empty / missing subcommand defaults to `"list"` (matches shipped
//! `"" | "list"` arm).

use std::sync::Arc;

use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Truncate a string to at most `max` bytes, appending "..." if truncated.
/// Safe for multi-byte UTF-8: always splits on a char boundary.
///
/// Preserved byte-for-byte from the pre-AGS-817 async
/// `handle_memory_command` helper. Private to this module.
fn truncate_str(s: &str, max: usize) -> String {
    let trimmed = s.replace('\n', " ");
    if trimmed.len() <= max {
        trimmed
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

/// Zero-sized handler registered as the primary `/memory` command.
///
/// Aliases: `["mem"]` — PRESERVED from the shipped declare_handler! stub
/// (shipped-wins drift-reconcile; see module rustdoc Aliases section).
///
/// Subcommands dispatched inside `execute`:
/// * `""` / `list` — call `MemoryTrait::list_recent(10)` and emit
///   formatted recent-memories list (or empty/error branch).
/// * `search <query>` — call `MemoryTrait::recall_memories(query, 10)`
///   and emit formatted results (or usage/empty/error branch).
/// * `clear` — call `MemoryTrait::clear_all()` and emit count (or error).
/// * any other token — emit unknown-subcommand hint.
pub(crate) struct MemoryHandler;

impl CommandHandler for MemoryHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // 1. Require memory handle. `build_command_context` populates
        //    this unconditionally from `SlashCommandContext::memory` so
        //    at the real dispatch site this branch never fires. Test
        //    fixtures that construct `CommandContext` directly with
        //    `memory: None` will hit this branch and observe an
        //    Err — mirroring the AGS-815
        //    `fork_handler_execute_without_session_id_returns_err`
        //    pattern.
        let Some(memory_arc) = ctx.memory.as_ref() else {
            return Err(anyhow::anyhow!(
                "/memory dispatched without memory handle — \
                 CommandContext population missing in dispatch-site \
                 builder (build_command_context always populates it; \
                 this is a test-fixture or wiring bug)"
            ));
        };
        // Borrow through Arc<dyn MemoryTrait>. No clone needed — the
        // handler body never outlives `ctx`.
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        // 2. Args-path reconciliation: shipped body used
        //    `input.split_once(' ')` on the whole rest-string; the
        //    parser instead hands us a tokenized args vec. Rebuild the
        //    shipped single-string arg by joining tokens with ' '.
        //    See module rustdoc "Args-path reconciliation" section.
        let subcmd = args
            .first()
            .map(|s| s.as_str())
            .unwrap_or("")
            .trim();
        let arg_joined = args
            .get(1..)
            .map(|rest| rest.join(" "))
            .unwrap_or_default();
        let arg = arg_joined.trim();

        match subcmd {
            "" | "list" => match memory.list_recent(10) {
                Ok(memories) if memories.is_empty() => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                        "\nNo memories stored.\n".into(),
                    ));
                }
                Ok(memories) => {
                    let mut out =
                        format!("\nRecent memories ({}):\n", memories.len());
                    for m in &memories {
                        let short_id = &m.id[..8.min(m.id.len())];
                        let date = m.created_at.format("%Y-%m-%d %H:%M");
                        out.push_str(&format!(
                            "  [{short_id}] {title} ({mtype}, {date})\n",
                            title = m.title,
                            mtype = m.memory_type,
                        ));
                    }
                    let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(out));
                }
                Err(e) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                        "Memory graph error: {e}"
                    )));
                }
            },
            "search" => {
                if arg.is_empty() {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                        "Usage: /memory search <query>".into(),
                    ));
                    return Ok(());
                }
                match memory.recall_memories(arg, 10) {
                    Ok(results) if results.is_empty() => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                            format!("\nNo memories matching \"{arg}\".\n"),
                        ));
                    }
                    Ok(results) => {
                        let mut out = format!(
                            "\nMemories matching \"{arg}\" ({}):\n",
                            results.len()
                        );
                        for m in &results {
                            let short_id = &m.id[..8.min(m.id.len())];
                            out.push_str(&format!(
                                "  [{short_id}] {title} -- {snippet}\n",
                                title = m.title,
                                snippet = truncate_str(&m.content, 80),
                            ));
                        }
                        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(out));
                    }
                    Err(e) => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                            format!("Memory search error: {e}"),
                        ));
                    }
                }
            }
            "clear" => match memory.clear_all() {
                Ok(n) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                        "\nCleared {n} memories from the graph.\n"
                    )));
                }
                Err(e) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                        "Failed to clear memories: {e}"
                    )));
                }
            },
            other => {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                    "Unknown memory subcommand: {other}. Use list, \
                     search, or clear."
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Preserved from the shipped declare_handler! stub at
        // registry.rs:522 (shipped-wins drift-reconcile).
        "Inspect or manage long-term memory"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved from the shipped declare_handler! stub (see module
        // rustdoc Aliases section).
        &["mem"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-817: tests for /memory slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};
    use archon_tui::app::TuiEvent;
    use chrono::{TimeZone, Utc};
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Inline TestMemory double used by the AGS-817 tests.
    ///
    /// `archon_test_support::memory::MockMemoryTrait` exists but every
    /// non-store method panics with `unimplemented!()`. The AGS-817
    /// tests exercise `list_recent`, `recall_memories`, and `clear_all`
    /// — so we define a local double that returns configurable
    /// pre-canned values. Defined here rather than extending the shared
    /// mock so the AGS-817 blast radius stays scoped to this file.
    struct TestMemory {
        list_recent_result: Mutex<Result<Vec<Memory>, MemoryError>>,
        recall_result: Mutex<Result<Vec<Memory>, MemoryError>>,
        clear_result: Mutex<Result<usize, MemoryError>>,
        recall_captured_query: Mutex<Option<String>>,
    }

    impl TestMemory {
        fn new() -> Self {
            Self {
                list_recent_result: Mutex::new(Ok(Vec::new())),
                recall_result: Mutex::new(Ok(Vec::new())),
                clear_result: Mutex::new(Ok(0)),
                recall_captured_query: Mutex::new(None),
            }
        }

        fn with_list_recent(
            self,
            r: Result<Vec<Memory>, MemoryError>,
        ) -> Self {
            *self.list_recent_result.lock().unwrap() = r;
            self
        }

        fn with_recall(self, r: Result<Vec<Memory>, MemoryError>) -> Self {
            *self.recall_result.lock().unwrap() = r;
            self
        }

        fn with_clear(self, r: Result<usize, MemoryError>) -> Self {
            *self.clear_result.lock().unwrap() = r;
            self
        }

        fn captured_recall_query(&self) -> Option<String> {
            self.recall_captured_query.lock().unwrap().clone()
        }
    }

    // Clone `MemoryError` by round-tripping via Display — MemoryError
    // doesn't derive Clone in the shipped types, but our test doubles
    // need to return the same Err on repeat calls. We box the Display
    // form into the Database variant (arbitrarily chosen since AGS-817
    // tests don't exercise the error-variant-distinguishing path). Only
    // used internally to this test module.
    fn clone_result<T: Clone>(
        r: &Result<T, MemoryError>,
    ) -> Result<T, MemoryError> {
        match r {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MemoryError::Database(format!("{e}"))),
        }
    }

    impl MemoryTrait for TestMemory {
        fn store_memory(
            &self,
            _content: &str,
            _title: &str,
            _memory_type: MemoryType,
            _importance: f64,
            _tags: &[String],
            _source_type: &str,
            _project_path: &str,
        ) -> Result<String, MemoryError> {
            unimplemented!("TestMemory: store_memory not used by AGS-817 tests")
        }

        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            unimplemented!("TestMemory: get_memory not used by AGS-817 tests")
        }

        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), MemoryError> {
            unimplemented!("TestMemory: update_memory not used by AGS-817 tests")
        }

        fn update_importance(
            &self,
            _id: &str,
            _importance: f64,
        ) -> Result<(), MemoryError> {
            unimplemented!("TestMemory: update_importance not used by AGS-817 tests")
        }

        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
            unimplemented!("TestMemory: delete_memory not used by AGS-817 tests")
        }

        fn create_relationship(
            &self,
            _from_id: &str,
            _to_id: &str,
            _rel_type: RelType,
            _context: Option<&str>,
            _strength: f64,
        ) -> Result<(), MemoryError> {
            unimplemented!(
                "TestMemory: create_relationship not used by AGS-817 tests"
            )
        }

        fn recall_memories(
            &self,
            query: &str,
            _limit: usize,
        ) -> Result<Vec<Memory>, MemoryError> {
            *self.recall_captured_query.lock().unwrap() =
                Some(query.to_string());
            clone_result(&self.recall_result.lock().unwrap())
        }

        fn search_memories(
            &self,
            _filter: &SearchFilter,
        ) -> Result<Vec<Memory>, MemoryError> {
            unimplemented!("TestMemory: search_memories not used by AGS-817 tests")
        }

        fn list_recent(
            &self,
            _limit: usize,
        ) -> Result<Vec<Memory>, MemoryError> {
            clone_result(&self.list_recent_result.lock().unwrap())
        }

        fn memory_count(&self) -> Result<usize, MemoryError> {
            unimplemented!("TestMemory: memory_count not used by AGS-817 tests")
        }

        fn clear_all(&self) -> Result<usize, MemoryError> {
            clone_result(&self.clear_result.lock().unwrap())
        }

        fn get_related_memories(
            &self,
            _id: &str,
            _depth: u32,
        ) -> Result<Vec<Memory>, MemoryError> {
            unimplemented!(
                "TestMemory: get_related_memories not used by AGS-817 tests"
            )
        }
    }

    /// Build a `Memory` record for use in list / search test fixtures.
    fn make_mem(id: &str, title: &str, content: &str) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            title: title.to_string(),
            memory_type: MemoryType::Fact,
            importance: 0.5,
            tags: Vec::new(),
            source_type: "test".to_string(),
            project_path: "/tmp/test".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
            updated_at: None,
            access_count: 0,
            last_accessed: None,
        }
    }

    /// Build a `CommandContext` backed by a fresh mpsc channel and the
    /// supplied `memory` handle. Mirrors the `make_ctx` fixtures in
    /// fork.rs / voice.rs / hooks.rs.
    ///
    /// Every optional field other than `memory` stays `None` — `/memory`
    /// is a DIRECT-pattern handler and does not consume any of the
    /// typed snapshots.
    fn make_ctx(
        memory: Option<Arc<dyn MemoryTrait>>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                context_snapshot: None,
                session_id: None,
                memory,
                // TASK-AGS-POST-6-BODIES-B01-FAST: /memory tests never exercise /fast paths — None.
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /memory tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /memory tests never exercise /diff paths — None.
                working_dir: None,
                pending_effect: None,
            },
            rx,
        )
    }

    #[test]
    fn memory_handler_description_matches() {
        let h = MemoryHandler;
        assert_eq!(
            h.description(),
            "Inspect or manage long-term memory",
            "MemoryHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn memory_handler_aliases_preserve_mem() {
        let h = MemoryHandler;
        assert_eq!(
            h.aliases(),
            &["mem"],
            "MemoryHandler aliases must preserve 'mem' from the shipped \
             declare_handler! stub (shipped-wins drift-reconcile — \
             dropping it would regress operators using /mem today)"
        );
    }

    /// When `CommandContext::memory` is `None`, execute() must return
    /// Err describing the missing field. The real builder populates
    /// the field unconditionally; this branch guards against test-
    /// fixture or wiring regressions. Mirrors AGS-815 fork Err path.
    #[test]
    fn memory_handler_execute_without_memory_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "MemoryHandler::execute must return Err when memory is None \
             (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("dispatched without memory"),
            "Err message must mention 'dispatched without memory' so \
             the operator can trace the wiring bug, got: {msg}"
        );
    }

    #[test]
    fn memory_handler_execute_list_empty_emits_no_memories_stored() {
        let mem: Arc<dyn MemoryTrait> =
            Arc::new(TestMemory::new().with_list_recent(Ok(Vec::new())));
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "list(empty) must return Ok, got: {res:?}");

        let mut saw_empty = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text == "\nNo memories stored.\n" {
                    saw_empty = true;
                }
            }
        }
        assert!(
            saw_empty,
            "MemoryHandler::execute(list empty) must emit the byte-for-\
             byte TextDelta '\\nNo memories stored.\\n'"
        );
    }

    #[test]
    fn memory_handler_execute_list_with_results_emits_recent_memories() {
        let m1 = make_mem("abcd1234-aaaa", "first title", "first content");
        let m2 = make_mem("efgh5678-bbbb", "second title", "second content");
        let mem: Arc<dyn MemoryTrait> = Arc::new(
            TestMemory::new().with_list_recent(Ok(vec![m1, m2])),
        );
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok(), "list(with results) must return Ok, got: {res:?}");

        let mut got_delta: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                got_delta = Some(text);
            }
        }
        let text = got_delta.expect(
            "MemoryHandler::execute(list with results) must emit a \
             TextDelta event",
        );
        assert!(
            text.contains("Recent memories (2):"),
            "TextDelta must contain 'Recent memories (2):' header, got: \
             {text}"
        );
        assert!(
            text.contains("[abcd1234]"),
            "TextDelta must contain short id of first memory \
             '[abcd1234]', got: {text}"
        );
        assert!(
            text.contains("[efgh5678]"),
            "TextDelta must contain short id of second memory \
             '[efgh5678]', got: {text}"
        );
        assert!(
            text.contains("first title"),
            "TextDelta must include first memory title, got: {text}"
        );
        assert!(
            text.contains("second title"),
            "TextDelta must include second memory title, got: {text}"
        );
    }

    #[test]
    fn memory_handler_execute_search_empty_query_emits_usage_error() {
        let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new());
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &["search".to_string()]);
        assert!(res.is_ok(), "search(empty) must return Ok, got: {res:?}");

        let mut saw_usage = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev {
                if text == "Usage: /memory search <query>" {
                    saw_usage = true;
                }
            }
        }
        assert!(
            saw_usage,
            "MemoryHandler::execute(search empty) must emit \
             TuiEvent::Error with the byte-for-byte usage hint"
        );
    }

    #[test]
    fn memory_handler_execute_search_with_query_joins_multi_token_args() {
        let tm =
            TestMemory::new().with_recall(Ok(vec![make_mem(
                "fff11111-gggg",
                "hello-world-memory",
                "a matching content snippet",
            )]));
        let tm_arc = Arc::new(tm);
        let mem: Arc<dyn MemoryTrait> = tm_arc.clone();
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(
            &mut ctx,
            &[
                "search".to_string(),
                "hello".to_string(),
                "world".to_string(),
            ],
        );
        assert!(
            res.is_ok(),
            "search(multi-token) must return Ok, got: {res:?}"
        );
        // Args-reconciliation assertion: the handler must have rebuilt
        // the shipped single-string semantics by joining tokens with ' '.
        let captured = tm_arc.captured_recall_query();
        assert_eq!(
            captured.as_deref(),
            Some("hello world"),
            "MemoryHandler::execute(search hello world) must forward \
             'hello world' as a single joined query (shipped split_once \
             semantics preserved)"
        );
        // Output assertion.
        let mut got_delta: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                got_delta = Some(text);
            }
        }
        let text = got_delta
            .expect("search(with results) must emit a TextDelta event");
        assert!(
            text.contains("Memories matching \"hello world\" (1):"),
            "TextDelta must contain the result-count header, got: {text}"
        );
        assert!(
            text.contains("[fff11111]"),
            "TextDelta must contain short id, got: {text}"
        );
        assert!(
            text.contains("hello-world-memory"),
            "TextDelta must contain the memory title, got: {text}"
        );
    }

    #[test]
    fn memory_handler_execute_search_empty_results_emits_no_match() {
        let mem: Arc<dyn MemoryTrait> =
            Arc::new(TestMemory::new().with_recall(Ok(Vec::new())));
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(
            &mut ctx,
            &["search".to_string(), "missing-token".to_string()],
        );
        assert!(
            res.is_ok(),
            "search(empty results) must return Ok, got: {res:?}"
        );
        let mut saw_no_match = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text
                    == "\nNo memories matching \"missing-token\".\n"
                {
                    saw_no_match = true;
                }
            }
        }
        assert!(
            saw_no_match,
            "MemoryHandler::execute(search no-match) must emit the \
             byte-for-byte 'No memories matching' TextDelta"
        );
    }

    #[test]
    fn memory_handler_execute_clear_emits_cleared_count() {
        let mem: Arc<dyn MemoryTrait> =
            Arc::new(TestMemory::new().with_clear(Ok(7)));
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &["clear".to_string()]);
        assert!(res.is_ok(), "clear must return Ok, got: {res:?}");
        let mut saw_cleared = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text == "\nCleared 7 memories from the graph.\n" {
                    saw_cleared = true;
                }
            }
        }
        assert!(
            saw_cleared,
            "MemoryHandler::execute(clear) must emit the byte-for-byte \
             '\\nCleared 7 memories from the graph.\\n' TextDelta"
        );
    }

    #[test]
    fn memory_handler_execute_unknown_subcommand_emits_error() {
        let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new());
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(&mut ctx, &["nope".to_string()]);
        assert!(
            res.is_ok(),
            "unknown subcommand must return Ok, got: {res:?}"
        );
        let mut saw_unknown = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev {
                if text
                    == "Unknown memory subcommand: nope. Use list, \
                        search, or clear."
                {
                    saw_unknown = true;
                }
            }
        }
        assert!(
            saw_unknown,
            "MemoryHandler::execute(unknown) must emit the byte-for-byte \
             'Unknown memory subcommand: nope. Use list, search, or \
             clear.' TuiEvent::Error"
        );
    }

    /// The `truncate_str` helper must split safely on UTF-8 char
    /// boundaries. Guards against regression of the
    /// `is_char_boundary` check when the byte-slice fallthrough point
    /// lands inside a multi-byte character. Preserved invariant from
    /// the pre-migration module.
    #[test]
    fn truncate_str_respects_utf8_char_boundaries() {
        // Three-byte emoji-ish char (U+4E2D zh "middle") repeated.
        let s = "中".repeat(40); // 40 * 3 = 120 bytes > 80
        let out = truncate_str(&s, 80);
        assert!(
            out.ends_with("..."),
            "truncate_str must append '...' when exceeded, got: {out}"
        );
        // Must not panic and must produce valid UTF-8.
        assert!(out.is_char_boundary(out.len()));
    }
}
