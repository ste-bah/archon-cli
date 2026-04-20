//! TASK-AGS-POST-6-BODIES-B13-GARDEN: /garden slash-command handler
//! (DIRECT-sync-via-MemoryTrait pattern, body-migrated from the legacy
//! match arm at `src/command/slash.rs:124-155` and the `declare_handler!`
//! stub at `src/command/registry.rs:958`).
//!
//! # Pattern reclassification — DIRECT-sync-via-MemoryTrait (NOT SNAPSHOT-async)
//!
//! The original B13 ticket label read "SNAPSHOT async" but that
//! classification was mechanical, not actual. Both underlying
//! archon-memory entry points are fully SYNC:
//!
//!   * `archon_memory::garden::format_garden_stats(&dyn MemoryTrait,
//!     usize) -> Result<String, MemoryError>`
//!   * `archon_memory::garden::consolidate(&dyn MemoryTrait,
//!     &GardenConfig) -> Result<GardenReport, MemoryError>`
//!
//! Neither function is `async`, neither takes a future, and both operate
//! on a plain `&dyn MemoryTrait` borrow (all 12 MemoryTrait methods are
//! sync). This matches the AGS-817 `/memory` DIRECT-pattern precedent at
//! `src/command/memory.rs` verbatim — no SNAPSHOT type, no async
//! mutex-lock traffic inside the builder, no `CommandEffect` variant.
//!
//! # R-item inventory (mirror of memory.rs R-items)
//!
//! * **R1 — description/aliases pinned.** Description
//!   "Run memory garden consolidation or show stats" is preserved
//!   byte-for-byte from the shipped `declare_handler!` stub at
//!   `registry.rs:958`. Aliases list is empty (`&[]`) to match the
//!   shipped two-arg declare_handler! form (no aliases slice). Shipped-
//!   wins drift-reconcile rule per AGS-817.
//!
//! * **R2 — try_send ergonomics.** The shipped legacy arm used
//!   `tui_tx.send(..).await` (async). The sync `CommandHandler::execute`
//!   signature forbids `.await`, so every emission is switched to
//!   `ctx.tui_tx.try_send(..)` with the Result discarded via `let _ =`.
//!   Mirrors AGS-817 /memory R2. Dropping a message under 16-cap
//!   channel backpressure is preferable to stalling the dispatcher
//!   (same trade-off as /memory; /garden output is best-effort
//!   informational UI).
//!
//! * **R3 — error-first-returns.** Missing `ctx.memory` or (for the
//!   consolidate path) missing `ctx.garden_config` returns
//!   `Err(anyhow::Error)` describing the wiring bug. At the real
//!   dispatch site `build_command_context` populates both
//!   UNCONDITIONALLY so this branch never fires in production;
//!   test-fixture and wiring regressions observe the explicit Err
//!   instead of a panic. Mirrors AGS-815 /fork + AGS-817 /memory
//!   builder-contract guards.
//!
//! * **R4 — args-path reconciliation.** The shipped match arm peeled
//!   the subcommand off the raw input string via
//!   `s.strip_prefix("/garden").unwrap_or("").trim()`. The registry
//!   parser tokenises on whitespace before dispatch, so the handler
//!   receives `args: &[String]` where the subcommand (if any) is the
//!   first token. `args.first().map(|s| s.as_str()).unwrap_or("").trim()`
//!   reconstructs the shipped semantics exactly: missing first token
//!   (empty args) maps to `""` which falls into the default-consolidate
//!   branch, and `args[0] == "stats"` fires the stats branch. Mirrors
//!   AGS-817 /memory R4.
//!
//! * **R5 — no snapshot / no effect-slot required.** Unlike /status
//!   (AGS-807), /model (AGS-808), /cost (AGS-809), /mcp (AGS-811),
//!   /context (AGS-814), /denials (B08), /effort (B11), or
//!   /permissions (B12) — all of which required snapshot pre-capture
//!   or effect-slot deferral for async operations — /garden's two
//!   archon-memory entry points are pure sync and run directly inside
//!   `execute`. No `CommandContext::garden_snapshot` field is added.
//!   The `CommandEffect` enum is NOT extended. Only a
//!   `CommandContext::garden_config` DIRECT field is added (mirrors
//!   AGS-817 `CommandContext::memory`) so the consolidate path can
//!   reach the `&GardenConfig` borrow without crossing the
//!   `SlashCommandContext` boundary.
//!
//! * **R6 — emission ordering swap vs. shipped.** The shipped legacy
//!   arm used `tui_tx.send(..).await` so the emission completes
//!   (post-await) before the match arm returns. The sync handler
//!   uses `try_send` which returns synchronously — the emission
//!   enters the channel immediately and the handler returns `Ok(())`
//!   without waiting for the TUI event loop to drain. From the TUI's
//!   perspective, ordering of observed events is unchanged (events are
//!   read in the order they were pushed into the channel); only the
//!   handler-side timing is different. Mirrors B10/B11/B12 precedent
//!   for the async->sync emission swap.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the legacy match arm at
//! `slash.rs:124-155`:
//!   * `"stats"` Ok -> `TextDelta(format!("\n{stats}\n"))` via `try_send`
//!   * `"stats"` Err -> `Error(format!("Garden stats failed: {e}"))` via `try_send`
//!   * default Ok -> `TextDelta(format!("\n{formatted}\n"))` where
//!     `formatted = report.format()` via `try_send`
//!   * default Err -> `Error(format!("Garden consolidation failed: {e}"))` via `try_send`
//!
//! Leading AND trailing newlines are preserved (shipped used
//! `format!("\n{stats}\n")` and `format!("\n{formatted}\n")`).

use std::sync::Arc;

use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/garden` command.
///
/// Aliases: `[]` — PRESERVED from the shipped declare_handler! stub at
/// `registry.rs:958` (shipped-wins drift-reconcile; the stub used the
/// two-arg declare_handler! form with no aliases slice).
///
/// Subcommands dispatched inside `execute`:
/// * `"stats"` — call `archon_memory::garden::format_garden_stats(memory, 10)`
///   and emit formatted stats (or error branch).
/// * any other token (including empty) — call
///   `archon_memory::garden::consolidate(memory, &garden_config)` and
///   emit the formatted report (or error branch).
pub(crate) struct GardenHandler;

impl CommandHandler for GardenHandler {
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
        //    Err — mirroring the AGS-817 /memory pattern.
        let Some(memory_arc): Option<&Arc<dyn MemoryTrait>> = ctx.memory.as_ref() else {
            return Err(anyhow::anyhow!(
                "/garden dispatched without memory handle — \
                 CommandContext population missing in dispatch-site \
                 builder (build_command_context always populates it; \
                 this is a test-fixture or wiring bug)"
            ));
        };
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        // 2. Args-path reconciliation. Shipped legacy arm used
        //    `s.strip_prefix("/garden").unwrap_or("").trim()`. The
        //    registry parser tokenises on whitespace, so `args` is
        //    already a `Vec<String>` of individual tokens. We read
        //    `args.first()` as the subcommand (trimmed) — missing token
        //    defaults to `""`, which falls into the default consolidate
        //    branch below. See module rustdoc R4.
        let sub = args
            .first()
            .map(|s| s.as_str())
            .unwrap_or("")
            .trim();

        if sub == "stats" {
            match archon_memory::garden::format_garden_stats(memory, 10) {
                Ok(stats) => {
                    let _ = ctx
                        .tui_tx
                        .try_send(TuiEvent::TextDelta(format!("\n{stats}\n")));
                }
                Err(e) => {
                    let _ = ctx
                        .tui_tx
                        .try_send(TuiEvent::Error(format!(
                            "Garden stats failed: {e}"
                        )));
                }
            }
        } else {
            // Consolidate path — requires `ctx.garden_config`. Same
            // builder-contract guard as `ctx.memory` above.
            let Some(garden_config) = ctx.garden_config.as_ref() else {
                return Err(anyhow::anyhow!(
                    "/garden dispatched without garden_config — \
                     CommandContext population missing in dispatch-site \
                     builder (build_command_context always populates it; \
                     this is a test-fixture or wiring bug)"
                ));
            };
            match archon_memory::garden::consolidate(memory, garden_config) {
                Ok(report) => {
                    let formatted = report.format();
                    let _ = ctx
                        .tui_tx
                        .try_send(TuiEvent::TextDelta(format!(
                            "\n{formatted}\n"
                        )));
                }
                Err(e) => {
                    let _ = ctx
                        .tui_tx
                        .try_send(TuiEvent::Error(format!(
                            "Garden consolidation failed: {e}"
                        )));
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Preserved byte-for-byte from the shipped declare_handler! stub
        // at registry.rs:958 (shipped-wins drift-reconcile).
        "Run memory garden consolidation or show stats"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved from the shipped declare_handler! stub (two-arg
        // form, no aliases slice). See module rustdoc R1.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B13-GARDEN: tests for /garden body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::garden::GardenConfig;
    use archon_memory::types::{
        Memory, MemoryError, MemoryType, RelType, SearchFilter,
    };
    use archon_tui::app::TuiEvent;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Inline TestMemory double used by the B13 tests.
    ///
    /// `archon_test_support::memory::MockMemoryTrait` exists but every
    /// non-store method panics with `unimplemented!()`. B13 needs a
    /// memory double that (a) answers `format_garden_stats(memory, 10)`
    /// deterministically and (b) lets `consolidate(memory, &config)`
    /// complete end-to-end without panicking. The consolidate path
    /// calls `memory_count`, `list_recent`, `search_memories`,
    /// `store_memory`, and optionally the update/decay/delete methods;
    /// on a fully-empty graph we only need the first four to return
    /// `Ok(empty)` / `Ok(0)` and the remainder are never reached. Each
    /// method has a configurable result slot so error-path tests can
    /// force a deterministic `MemoryError`.
    ///
    /// Defined here rather than extending the shared mock so the B13
    /// blast radius stays scoped to this file (matches AGS-817
    /// /memory TestMemory precedent).
    struct TestMemory {
        count_result: Mutex<Result<usize, MemoryError>>,
        list_recent_result: Mutex<Result<Vec<Memory>, MemoryError>>,
        search_result: Mutex<Result<Vec<Memory>, MemoryError>>,
        store_result: Mutex<Result<String, MemoryError>>,
    }

    impl TestMemory {
        fn new_empty() -> Self {
            Self {
                count_result: Mutex::new(Ok(0)),
                list_recent_result: Mutex::new(Ok(Vec::new())),
                search_result: Mutex::new(Ok(Vec::new())),
                store_result: Mutex::new(Ok("stored-id".to_string())),
            }
        }

        /// Force every observable entry point to return the same error
        /// (Database variant). `format_garden_stats` calls
        /// `memory_count` first, so driving that slot to Err is
        /// sufficient to exercise the stats-error path. Consolidate
        /// also calls `memory_count` first for `total_before`, so the
        /// same slot covers both paths.
        fn with_count_error(self, msg: &str) -> Self {
            *self.count_result.lock().unwrap() =
                Err(MemoryError::Database(msg.to_string()));
            self
        }
    }

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
            clone_result(&self.store_result.lock().unwrap())
        }

        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            unimplemented!("TestMemory: get_memory not used by B13 tests")
        }

        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), MemoryError> {
            Ok(())
        }

        fn update_importance(
            &self,
            _id: &str,
            _importance: f64,
        ) -> Result<(), MemoryError> {
            Ok(())
        }

        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
            Ok(())
        }

        fn create_relationship(
            &self,
            _from_id: &str,
            _to_id: &str,
            _rel_type: RelType,
            _context: Option<&str>,
            _strength: f64,
        ) -> Result<(), MemoryError> {
            Ok(())
        }

        fn recall_memories(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<Memory>, MemoryError> {
            Ok(Vec::new())
        }

        fn search_memories(
            &self,
            _filter: &SearchFilter,
        ) -> Result<Vec<Memory>, MemoryError> {
            clone_result(&self.search_result.lock().unwrap())
        }

        fn list_recent(
            &self,
            _limit: usize,
        ) -> Result<Vec<Memory>, MemoryError> {
            clone_result(&self.list_recent_result.lock().unwrap())
        }

        fn memory_count(&self) -> Result<usize, MemoryError> {
            match &*self.count_result.lock().unwrap() {
                Ok(n) => Ok(*n),
                // Unwrap the inner message instead of re-wrapping via
                // `format!("{e}")` — the thiserror Display impl on
                // MemoryError::Database prefixes "database error: ",
                // so naively round-tripping through Display and
                // re-wrapping would double-prefix and break the
                // byte-identity assertions.
                Err(MemoryError::Database(msg)) => {
                    Err(MemoryError::Database(msg.clone()))
                }
                Err(other) => Err(MemoryError::Database(format!("{other}"))),
            }
        }

        fn clear_all(&self) -> Result<usize, MemoryError> {
            Ok(0)
        }

        fn get_related_memories(
            &self,
            _id: &str,
            _depth: u32,
        ) -> Result<Vec<Memory>, MemoryError> {
            Ok(Vec::new())
        }
    }

    /// Build a `CommandContext` backed by a fresh mpsc channel and the
    /// supplied `memory` / `garden_config` handles. Mirrors the
    /// `make_ctx` fixtures in memory.rs / fork.rs. Every optional field
    /// other than `memory` / `garden_config` stays `None` — /garden is
    /// a DIRECT-pattern handler and does not consume any of the typed
    /// snapshots.
    fn make_ctx(
        memory: Option<Arc<dyn MemoryTrait>>,
        garden_config: Option<GardenConfig>,
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
                garden_config,
                fast_mode_shared: None,
                show_thinking: None,
                working_dir: None,
                skill_registry: None,
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    // ---------------------------------------------------------------
    // R1: description + aliases byte-identity tests
    // ---------------------------------------------------------------

    #[test]
    fn garden_handler_description_byte_identical_to_shipped() {
        let h = GardenHandler;
        assert_eq!(
            h.description(),
            "Run memory garden consolidation or show stats",
            "GardenHandler description must match the shipped \
             declare_handler! stub at registry.rs:958 byte-for-byte \
             (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn garden_handler_aliases_are_empty() {
        let h = GardenHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "GardenHandler aliases must be empty to match the shipped \
             declare_handler! stub (two-arg form, no aliases slice)"
        );
    }

    // ---------------------------------------------------------------
    // R3: missing-memory Err branch
    // ---------------------------------------------------------------

    /// When `CommandContext::memory` is `None`, execute() must return
    /// Err describing the missing field. Production builder populates
    /// the field unconditionally; this branch guards against test-
    /// fixture or wiring regressions. Mirrors AGS-817 /memory Err path.
    #[test]
    fn garden_handler_execute_without_memory_handle_returns_err() {
        let (mut ctx, _rx) = make_ctx(None, Some(GardenConfig::default()));
        let h = GardenHandler;
        let res = h.execute(&mut ctx, &["stats".to_string()]);
        assert!(
            res.is_err(),
            "GardenHandler::execute must return Err when memory is None \
             (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.to_lowercase().contains("memory"),
            "Err message must mention 'memory' for operator traceability, \
             got: {msg}"
        );
        assert!(
            msg.contains("wiring") || msg.contains("builder"),
            "Err message must mention 'wiring' or 'builder' to locate \
             the fix site, got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // Stats branch — Ok + Err paths
    // ---------------------------------------------------------------

    /// Stats Ok path: `format_garden_stats(memory, 10)` on an empty
    /// memory store produces a deterministic string header. The
    /// handler wraps it in `format!("\n{stats}\n")` before emission.
    /// We assert the TextDelta bytes start with `"\n"` and contain the
    /// shipped header line so the wrapping invariant is pinned.
    #[test]
    fn garden_handler_execute_stats_with_ok_memory_emits_formatted_stats() {
        let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
        // Pre-compute expected payload by calling format_garden_stats
        // directly on the same memory double. This guarantees the
        // assertion stays in lockstep with the archon-memory formatter
        // across future changes without hard-coding its exact output.
        let expected_inner =
            archon_memory::garden::format_garden_stats(mem.as_ref(), 10)
                .expect("format_garden_stats on empty TestMemory must succeed");
        let expected = format!("\n{expected_inner}\n");

        let (mut ctx, mut rx) =
            make_ctx(Some(mem), Some(GardenConfig::default()));
        let h = GardenHandler;
        let res = h.execute(&mut ctx, &["stats".to_string()]);
        assert!(res.is_ok(), "stats Ok must return Ok, got: {res:?}");

        let mut got: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                got = Some(text);
            }
        }
        let text = got.expect(
            "stats Ok path must emit a TextDelta event with the formatted \
             stats payload",
        );
        assert_eq!(
            text, expected,
            "stats Ok payload must equal format!(\"\\n{{stats}}\\n\") \
             byte-for-byte (shipped legacy arm semantics)"
        );
    }

    /// Stats Err path: drive `memory_count` (the first call inside
    /// `format_garden_stats`) to return an error. The handler must
    /// emit `TuiEvent::Error(format!("Garden stats failed: {e}"))`.
    #[test]
    fn garden_handler_execute_stats_with_err_memory_emits_error() {
        let tm = TestMemory::new_empty().with_count_error("boom-stats");
        let mem: Arc<dyn MemoryTrait> = Arc::new(tm);
        let (mut ctx, mut rx) =
            make_ctx(Some(mem), Some(GardenConfig::default()));
        let h = GardenHandler;
        let res = h.execute(&mut ctx, &["stats".to_string()]);
        assert!(
            res.is_ok(),
            "stats Err branch must still return Ok (error is emitted \
             via TuiEvent::Error, not surfaced via Err), got: {res:?}"
        );

        let mut got: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev {
                got = Some(text);
            }
        }
        let text = got.expect(
            "stats Err path must emit a TuiEvent::Error with the 'Garden \
             stats failed' prefix",
        );
        // `format!("{e}")` on MemoryError::Database(msg) renders
        // "database error: {msg}" per the thiserror annotation.
        assert_eq!(
            text, "Garden stats failed: database error: boom-stats",
            "stats Err payload must equal format!(\"Garden stats \
             failed: {{e}}\") byte-for-byte (shipped legacy arm semantics)"
        );
    }

    // ---------------------------------------------------------------
    // Consolidate branch — Ok + Err paths
    // ---------------------------------------------------------------

    /// Consolidate Ok path: running the full six-phase consolidation
    /// over an empty memory graph produces a `GardenReport` with zero
    /// counts across every phase. The `duration_ms` field is
    /// non-deterministic but every OTHER field of the formatted output
    /// is stable, so we assert that the emitted TextDelta starts with
    /// "\n", ends with "\n", and contains the "Memory Garden —
    /// Consolidation Complete" header + "Before: 0 memories" / "After:
    /// 0 memories" lines produced by `GardenReport::format`.
    #[test]
    fn garden_handler_execute_consolidate_with_ok_memory_emits_formatted_report() {
        let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
        let cfg = GardenConfig::default();
        let (mut ctx, mut rx) = make_ctx(Some(mem), Some(cfg));
        let h = GardenHandler;
        // Default branch — empty args vec.
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "consolidate Ok must return Ok, got: {res:?}");

        let mut got: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                got = Some(text);
            }
        }
        let text = got.expect(
            "consolidate Ok path must emit a TextDelta event with the \
             formatted report payload",
        );
        // Leading + trailing newline invariant from
        // format!("\n{formatted}\n").
        assert!(
            text.starts_with('\n'),
            "consolidate Ok payload must start with '\\n' (shipped \
             format!(\"\\n{{formatted}}\\n\") invariant), got: {text:?}"
        );
        assert!(
            text.ends_with('\n'),
            "consolidate Ok payload must end with '\\n' (shipped \
             format!(\"\\n{{formatted}}\\n\") invariant), got: {text:?}"
        );
        // GardenReport::format header preservation.
        assert!(
            text.contains("Memory Garden — Consolidation Complete"),
            "consolidate Ok payload must contain the GardenReport \
             header 'Memory Garden — Consolidation Complete', got: \
             {text}"
        );
        // Empty-graph counters — deterministic under empty TestMemory.
        assert!(
            text.contains("Before: 0 memories"),
            "consolidate Ok payload must contain 'Before: 0 memories' \
             when running over an empty graph, got: {text}"
        );
        assert!(
            text.contains("After:  0 memories"),
            "consolidate Ok payload must contain 'After:  0 memories' \
             when running over an empty graph, got: {text}"
        );
        assert!(
            text.contains("Duplicates merged:    0"),
            "consolidate Ok payload must contain 'Duplicates merged: \
             0' row from GardenReport::format, got: {text}"
        );
    }

    /// Consolidate Err path: drive `memory_count` to return an error
    /// (consolidate calls `memory_count` first for `total_before`).
    /// The handler must emit
    /// `TuiEvent::Error(format!("Garden consolidation failed: {e}"))`.
    #[test]
    fn garden_handler_execute_consolidate_with_err_memory_emits_error() {
        let tm = TestMemory::new_empty().with_count_error("boom-consolidate");
        let mem: Arc<dyn MemoryTrait> = Arc::new(tm);
        let (mut ctx, mut rx) =
            make_ctx(Some(mem), Some(GardenConfig::default()));
        let h = GardenHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "consolidate Err branch must still return Ok (error is \
             emitted via TuiEvent::Error, not surfaced via Err), got: \
             {res:?}"
        );

        let mut got: Option<String> = None;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev {
                got = Some(text);
            }
        }
        let text = got.expect(
            "consolidate Err path must emit a TuiEvent::Error with the \
             'Garden consolidation failed' prefix",
        );
        assert_eq!(
            text,
            "Garden consolidation failed: database error: boom-consolidate",
            "consolidate Err payload must equal format!(\"Garden \
             consolidation failed: {{e}}\") byte-for-byte (shipped \
             legacy arm semantics)"
        );
    }
}
