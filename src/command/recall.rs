//! TASK-AGS-POST-6-BODIES-B18-RECALL: /recall slash-command handler
//! (DIRECT-sync-via-MemoryTrait pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1305` and the legacy match arm at
//! `src/command/slash.rs:569-615`.
//!
//! # R1 — pattern = DIRECT-sync-via-MemoryTrait (no snapshot, no effect slot)
//!
//! The shipped `/recall` body calls
//! `archon_memory::MemoryTrait::recall_memories(query, limit)` which
//! is a plain sync method on the object-safe trait (see
//! `crates/archon-memory/src/access.rs` — all 12 methods are sync and
//! the trait carries `Send + Sync`). No `tokio::sync::Mutex` guards on
//! the read path and no writes back to `SlashCommandContext` state.
//! Consequently:
//!
//! - NO `RecallSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits `TuiEvent`s — matches AGS-815 /fork precedent).
//! - NO `build_command_context` match arm added for `/recall`. Unlike
//!   the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which gate
//!   their populate step on the primary name, AGS-817 /memory already
//!   extended `CommandContext` with
//!   `memory: Option<Arc<dyn MemoryTrait>>` populated UNCONDITIONALLY
//!   in the builder (`context.rs:69` —
//!   `memory: Some(Arc::clone(&slash_ctx.memory))`). `/recall` reuses
//!   that exact field — no new context.rs wiring required for this
//!   ticket. Matches the cross-cutting precedent set by AGS-817 for
//!   `memory` and AGS-815 for `session_id`.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/recall` match arm at slash.rs:569-615 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying `recall_memories`
//! call is 100% sync. In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork + AGS-817 /memory precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used `s.strip_prefix("/recall").unwrap_or("").trim()`
//! on the full input string, so `/recall hello world` (two tokens) was
//! forwarded verbatim as the query `"hello world"`. The registry
//! parser tokenizes on whitespace, so `args` is `["hello", "world"]`.
//! To preserve the shipped single-string semantics while going through
//! the parser, the handler joins `args` with a single space then
//! `.trim()`s. This is byte-equivalent to the shipped behaviour for
//! all inputs: single-token queries pass through unchanged, multi-token
//! queries preserve the whitespace-joined substring, empty args →
//! empty string → usage-error branch identical to the shipped
//! `if query.is_empty()` check at slash.rs:572. See
//! `src/command/add_dir.rs:155-180` and `src/command/rename.rs:139-140`
//! for the same pattern.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Recall memories matching a query"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1305.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and the Steven
//!   directive at registry.rs:1302-1304 explicitly forbids adding
//!   `recall` as an alias on `/memory` or any other handler.
//! - Emitted events preserve the shipped slash.rs:569-615 format
//!   strings BYTE-FOR-BYTE, including the EM-DASH (U+2014, NOT a
//!   hyphen) in the empty-query usage error:
//!   * Empty-query → `TuiEvent::Error("Usage: /recall <query> — \
//!     search memories by keyword")`. The `—` character between
//!     `<query>` and `search` is Unicode EM DASH (U+2014), NOT a
//!     hyphen-minus. Any ASCII-ification here is a byte-identity
//!     violation and Sherlock will flag it.
//!   * No-match → `TuiEvent::TextDelta(format!("\nNo memories found \
//!     for '{query}'.\n"))` — literal single-quotes around `{query}`.
//!   * Match header → `format!("\n{} memories matching '{query}':\n\
//!     \n", memories.len())` — count FIRST, then word "memories",
//!     single-quotes around query, trailing colon + blank line.
//!   * Per-entry → `"  [{id_short}] {title}\n    {snippet}...\n\n"` —
//!     TWO spaces then bracket, ONE space between bracket and title,
//!     FOUR spaces before snippet, literal trailing `...` then blank
//!     line.
//!   * Title fallback → `if m.title.is_empty() { "(untitled)" } else
//!     { &m.title }` — parens around "untitled".
//!   * Snippet → `m.content.chars().take(100).collect::<String>()` —
//!     CHAR take (UTF-8 safe), NOT byte slice.
//!   * id_short → `&m.id[..8.min(m.id.len())]` — byte slice with
//!     length cap.
//!   * Recall limit → `10` (hardcoded).
//!   * Search failure → `TuiEvent::Error(format!("Memory search \
//!     failed: {e}"))`.
//!
//! # R5 — aliases = zero (Steven directive)
//!
//! Shipped pre-B18: none (2-arg declare_handler! form at
//! registry.rs:1305). The comment block at registry.rs:1302-1304
//! encodes the Steven directive explicitly:
//!
//! > "/recall stays a standalone primary command and has NO aliases
//! > — Steven directive. Do NOT add \"recall\" as an alias on
//! > /memory or any other handler."
//!
//! No aliases added. Matches /fork / /mcp / /context / /hooks /
//! /rename precedent.
//!
//! # R6 — memory field reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` is already
//! populated unconditionally by `build_command_context` per AGS-817
//! /memory (`context.rs:69` —
//! `memory: Some(Arc::clone(&slash_ctx.memory))`). This ticket REUSES
//! that exact field — there is no `recall_snapshot` type, no
//! context.rs match arm added, no new `build_command_context` wiring.
//! The test fixture helper (`make_recall_ctx`) mirrors the AGS-817
//! /memory `make_ctx(memory)` shape.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `RecallHandler` (PATH A,
//! via the dispatcher at slash.rs:46) AND the legacy
//! `s if s.starts_with("/recall")` match arm at slash.rs:569-615 are
//! live. Every `/recall` invocation therefore fires twice — once via
//! the handler and once via the legacy arm. This is the Stage-6
//! body-migrate protocol: Gate 5 deletes the legacy match arm in a
//! SEPARATE subsequent subagent run (NOT this subagent's
//! responsibility). Do NOT touch slash.rs in this ticket.

use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/recall` command.
///
/// No aliases. Shipped pre-B18 stub carried none (2-arg
/// declare_handler! form) and the Steven directive at
/// registry.rs:1302-1304 explicitly forbids adding `recall` as an
/// alias on any other handler. Matches /fork / /mcp / /context /
/// /hooks / /rename precedent.
pub(crate) struct RecallHandler;

impl RecallHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs:1363
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for RecallHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for RecallHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R3: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/recall").unwrap_or("").trim()`
        // for all inputs — single-token queries collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // queries preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the usage-error branch identical to the shipped
        // `if query.is_empty()` check at slash.rs:572.
        let joined = args.join(" ");
        let query = joined.trim();

        if query.is_empty() {
            // Empty-query branch — byte-for-byte preservation of
            // shipped format string at slash.rs:574-576. The `—`
            // between `<query>` and `search` is Unicode EM DASH
            // (U+2014), NOT a hyphen-minus. Do NOT ASCII-ify.
            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                "Usage: /recall <query> — search memories by keyword"
                    .into(),
            ));
            return Ok(());
        }

        // R6: require memory handle. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::memory` per
        // the AGS-817 /memory precedent (context.rs:69), so at the
        // real dispatch site this branch never fires. Test fixtures
        // that construct `CommandContext` directly with `memory: None`
        // will hit this branch and observe an Err — mirroring the
        // AGS-817 `memory_handler_execute_without_memory_returns_err`
        // pattern.
        let memory_arc = ctx.memory.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "RecallHandler invoked without ctx.memory populated — \
                 build_command_context bug"
            )
        })?;
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        match memory.recall_memories(query, 10) {
            Ok(memories) => {
                if memories.is_empty() {
                    // No-match branch — byte-for-byte preservation of
                    // shipped format string at slash.rs:585-587.
                    let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                        format!("\nNo memories found for '{query}'.\n"),
                    ));
                } else {
                    // Match branch — byte-for-byte preservation of
                    // shipped format loop at slash.rs:590-604.
                    let mut out = format!(
                        "\n{} memories matching '{query}':\n\n",
                        memories.len()
                    );
                    for m in &memories {
                        let title = if m.title.is_empty() {
                            "(untitled)"
                        } else {
                            &m.title
                        };
                        // Snippet: char-based take(100) is UTF-8
                        // safe (byte slice `&m.content[..100]` would
                        // panic on a non-char-boundary split).
                        let snippet: String =
                            m.content.chars().take(100).collect();
                        let id_short = &m.id[..8.min(m.id.len())];
                        out.push_str(&format!(
                            "  [{id_short}] {title}\n    {snippet}...\n\n"
                        ));
                    }
                    let _ =
                        ctx.tui_tx.try_send(TuiEvent::TextDelta(out));
                }
            }
            Err(e) => {
                // Search-failure branch — byte-for-byte preservation
                // of shipped format string at slash.rs:608-610.
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                    "Memory search failed: {e}"
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1305.
        "Recall memories matching a query"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice) and the Steven
        // directive at registry.rs:1302-1304 explicitly forbids
        // adding `recall` as an alias on any other handler.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B18-RECALL: tests for /recall slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use archon_memory::types::{
        Memory, MemoryError, MemoryType, RelType, SearchFilter,
    };
    use archon_tui::app::TuiEvent;
    use chrono::{TimeZone, Utc};
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::{CommandContext, RegistryBuilder};

    /// Inline TestMemory double used by the B18 tests.
    ///
    /// Mirrors the AGS-817 /memory `TestMemory` pattern (scoped
    /// locally rather than extending `archon_test_support::memory::
    /// MockMemoryTrait` so the B18 blast radius stays in this file).
    /// Only `recall_memories` is exercised by `RecallHandler`; every
    /// other trait method panics with `unimplemented!()`.
    struct StubMemory {
        recall_result: Mutex<Result<Vec<Memory>, MemoryError>>,
        recall_captured_query: Mutex<Option<String>>,
    }

    impl StubMemory {
        fn new(result: Result<Vec<Memory>, MemoryError>) -> Self {
            Self {
                recall_result: Mutex::new(result),
                recall_captured_query: Mutex::new(None),
            }
        }

        fn captured_query(&self) -> Option<String> {
            self.recall_captured_query.lock().unwrap().clone()
        }
    }

    /// Clone a `Result<Vec<Memory>, MemoryError>` by round-tripping
    /// the error variant through Display (MemoryError doesn't derive
    /// Clone). Mirrors the AGS-817 /memory `clone_result` helper.
    fn clone_result(
        r: &Result<Vec<Memory>, MemoryError>,
    ) -> Result<Vec<Memory>, MemoryError> {
        match r {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MemoryError::Database(format!("{e}"))),
        }
    }

    impl MemoryTrait for StubMemory {
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
            unimplemented!("StubMemory: store_memory not used by B18 tests")
        }

        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            unimplemented!("StubMemory: get_memory not used by B18 tests")
        }

        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), MemoryError> {
            unimplemented!("StubMemory: update_memory not used by B18 tests")
        }

        fn update_importance(
            &self,
            _id: &str,
            _importance: f64,
        ) -> Result<(), MemoryError> {
            unimplemented!(
                "StubMemory: update_importance not used by B18 tests"
            )
        }

        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
            unimplemented!("StubMemory: delete_memory not used by B18 tests")
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
                "StubMemory: create_relationship not used by B18 tests"
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
            unimplemented!(
                "StubMemory: search_memories not used by B18 tests"
            )
        }

        fn list_recent(
            &self,
            _limit: usize,
        ) -> Result<Vec<Memory>, MemoryError> {
            unimplemented!("StubMemory: list_recent not used by B18 tests")
        }

        fn memory_count(&self) -> Result<usize, MemoryError> {
            unimplemented!(
                "StubMemory: memory_count not used by B18 tests"
            )
        }

        fn clear_all(&self) -> Result<usize, MemoryError> {
            unimplemented!("StubMemory: clear_all not used by B18 tests")
        }

        fn get_related_memories(
            &self,
            _id: &str,
            _depth: u32,
        ) -> Result<Vec<Memory>, MemoryError> {
            unimplemented!(
                "StubMemory: get_related_memories not used by B18 tests"
            )
        }
    }

    /// Build a `Memory` record for use in match-path test fixtures.
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
            created_at: Utc
                .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
                .unwrap(),
            updated_at: None,
            access_count: 0,
            last_accessed: None,
        }
    }

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `memory` handle. Mirrors the AGS-817 /memory
    /// `make_ctx(memory)` fixture — DIRECT pattern, no snapshot, no
    /// effect slot.
    fn make_recall_ctx(
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
                garden_config: None,
                fast_mode_shared: None,
                show_thinking: None,
                working_dir: None,
                skill_registry: None,
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                copy_snapshot: None,
                doctor_snapshot: None,
                usage_snapshot: None,
                config_path: None,
                auth_label: None,
                pending_effect: None,
                pending_effort_set: None,
                pending_export: None,
            },
            rx,
        )
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1305. Any drift here means the stub and
    /// the new handler have diverged — Sherlock will flag it.
    #[test]
    fn recall_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            RecallHandler::new().description(),
            "Recall memories matching a query"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice) and the Steven
    /// directive at registry.rs:1302-1304 explicitly forbids adding
    /// `recall` as an alias on any other handler.
    #[test]
    fn recall_handler_aliases_are_empty() {
        assert_eq!(RecallHandler::new().aliases(), &[] as &[&str]);
    }

    /// Empty args: emit the usage-error TuiEvent with the EXACT
    /// byte-identity em-dash literal (U+2014, NOT a hyphen) and
    /// return Ok(()). No memory lookup is performed (the empty-args
    /// branch short-circuits BEFORE the memory check, matching the
    /// shipped control flow at slash.rs:572).
    #[test]
    fn execute_with_empty_args_emits_usage_error_with_em_dash() {
        // memory: None is fine here — the empty-args branch
        // short-circuits before touching ctx.memory, matching shipped
        // control flow.
        let (mut ctx, mut rx) = make_recall_ctx(None);
        let h = RecallHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "empty-args branch must return Ok(()) (event emission is \
             best-effort via try_send), got: {res:?}"
        );
        let ev = rx.try_recv().expect("usage error must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg,
                    "Usage: /recall <query> — search memories by keyword",
                    "Usage error must be byte-identical to the shipped \
                     slash.rs:574-576 literal, INCLUDING the em-dash \
                     (U+2014) between '<query>' and 'search'. A \
                     hyphen-minus here is a byte-identity violation."
                );
                // Defence-in-depth: verify the em-dash byte-exactly.
                // U+2014 is 3 bytes in UTF-8: E2 80 94.
                assert!(
                    msg.contains('\u{2014}'),
                    "usage error must contain U+2014 EM DASH, got: {msg}"
                );
            }
            other => panic!(
                "expected TuiEvent::Error with em-dash usage literal, \
                 got: {other:?}"
            ),
        }
    }

    /// R6: when `memory` is None but args are non-empty, execute
    /// returns Err whose message mentions both `memory` and
    /// `build_command_context` so the operator can trace the wiring
    /// bug. Mirrors the AGS-817
    /// `memory_handler_execute_without_memory_returns_err` precedent.
    #[test]
    fn execute_without_memory_returns_err() {
        let (mut ctx, _rx) = make_recall_ctx(None);
        let h = RecallHandler::new();
        let res = h.execute(&mut ctx, &["myquery".to_string()]);
        assert!(
            res.is_err(),
            "RecallHandler::execute with None memory and non-empty \
             args must return Err (builder contract violation), \
             got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("memory"),
            "Err message must mention 'memory' so the operator can \
             trace the wiring bug, got: {msg}"
        );
        assert!(
            msg.contains("build_command_context"),
            "Err message must mention 'build_command_context' to pin \
             the owning builder, got: {msg}"
        );
    }

    /// Success path (match branch): stub returns one Memory and the
    /// handler must emit the byte-identical formatted-list TextDelta
    /// covering the complex per-entry format loop. Also asserts the
    /// query was forwarded verbatim to `recall_memories` (args
    /// reconciliation).
    #[test]
    fn execute_with_memory_and_matches_emits_formatted_list() {
        let stub = Arc::new(StubMemory::new(Ok(vec![make_mem(
            "abcdef1234",
            "Test Title",
            "hello world",
        )])));
        let memory: Arc<dyn MemoryTrait> = stub.clone();
        let (mut ctx, mut rx) = make_recall_ctx(Some(memory));
        let h = RecallHandler::new();
        let res = h.execute(&mut ctx, &["foo".to_string()]);
        assert!(
            res.is_ok(),
            "success path must return Ok(()), got: {res:?}"
        );

        // Args-reconciliation assertion: the handler must have
        // forwarded `"foo"` verbatim to `recall_memories`.
        assert_eq!(
            stub.captured_query().as_deref(),
            Some("foo"),
            "RecallHandler::execute(foo) must forward 'foo' verbatim \
             to recall_memories"
        );

        let ev = rx
            .try_recv()
            .expect("formatted-list TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text,
                    "\n1 memories matching 'foo':\n\n  [abcdef12] \
                     Test Title\n    hello world...\n\n",
                    "TextDelta must be byte-identical to the shipped \
                     slash.rs:590-604 format (count + word 'memories' \
                     + single-quoted query + colon + blank line + \
                     two-space bracket + one-space title + newline + \
                     four-space snippet + literal '...' + blank line)"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with formatted list, \
                 got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration (empty-arg short-circuit). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/recall` with
    /// `RecallHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/recall")` routes through
    /// the real alias+primary pipeline. Memory is None — the
    /// empty-arg branch short-circuits before touching memory so no
    /// stub is needed. Asserts the dispatcher's end-to-end wiring
    /// (parser → registry → handler.execute) delivers the byte-
    /// identical em-dash usage error.
    #[test]
    fn dispatcher_routes_slash_recall_with_empty_arg_emits_usage_error() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("recall", Arc::new(RecallHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_recall_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/recall");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for the empty-arg \
             short-circuit path, got: {res:?}"
        );

        let ev = rx.try_recv().expect("usage error must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg,
                    "Usage: /recall <query> — search memories by keyword",
                    "dispatcher must deliver the byte-identical \
                     em-dash usage error through the full parser → \
                     registry → handler pipeline"
                );
            }
            other => panic!(
                "expected TuiEvent::Error with em-dash usage literal, \
                 got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/recall`, dispatches
    /// `"/recall somequery"` with `memory: None`, and asserts that
    /// `Dispatcher::dispatch` surfaces the handler's Err
    /// (dispatcher.rs:110 forwards `handler.execute(..)` verbatim —
    /// it does NOT swallow handler-origin Errs). Mirrors the AGS-B17
    /// `dispatcher_routes_slash_rename_without_session_id_returns_err`
    /// precedent.
    #[test]
    fn dispatcher_routes_slash_recall_without_memory_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("recall", Arc::new(RecallHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_recall_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/recall somequery");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must surface handler Err when \
             memory is None (dispatcher forwards the Err verbatim), \
             got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("memory") && msg.contains("build_command_context"),
            "Err message must mention both 'memory' and \
             'build_command_context', got: {msg}"
        );
    }
}
