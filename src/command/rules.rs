//! TASK-AGS-POST-6-BODIES-B19-RULES: /rules slash-command handler
//! (DIRECT-sync-via-MemoryTrait pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1336` and the legacy match arm at
//! `src/command/slash.rs:591-706`.
//!
//! # R1 — pattern = DIRECT-sync-via-MemoryTrait (no snapshot, no effect slot)
//!
//! Despite the B19 task tag nominally listing "SNAPSHOT", recon proved
//! the correct pattern is identical to B18 /recall: the
//! `archon_consciousness::rules::RulesEngine::new(graph: &dyn MemoryTrait)`
//! constructor at `crates/archon-consciousness/src/rules.rs:112` takes a
//! plain `&dyn MemoryTrait` reference and every method exercised by
//! `/rules` is sync on the object-safe trait:
//!
//! * `get_rules_sorted(&self) -> Result<Vec<BehavioralRule>, RulesError>`
//!   at rules.rs:146 — calls `MemoryTrait::search_memories` (sync).
//! * `update_rule(&self, id: &str, text: &str) -> Result<(), RulesError>`
//!   at rules.rs:209 — calls `MemoryTrait::update_memory` (sync).
//! * `remove_rule(&self, id: &str) -> Result<(), RulesError>` at
//!   rules.rs:202 — calls `MemoryTrait::delete_memory` (sync).
//!
//! No `tokio::sync::Mutex` guards on the read path, no writes back to
//! `SlashCommandContext` state. Consequently:
//!
//! - NO `RulesSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state
//!   through the effect slot; rule-engine writes go through
//!   `MemoryTrait::update_memory`/`delete_memory` which are sync and
//!   already safe from a sync handler — same contract as B18 /recall).
//! - NO `build_command_context` match arm added for `/rules`. AGS-817
//!   already populates `CommandContext::memory:
//!   Option<Arc<dyn MemoryTrait>>` UNCONDITIONALLY in the builder
//!   (`context.rs:69` — `memory: Some(Arc::clone(&slash_ctx.memory))`).
//!   `/rules` reuses that exact field — no new context.rs wiring
//!   required for this ticket. Matches the cross-cutting precedent set
//!   by AGS-817 for `memory` and AGS-815 for `session_id`, and the B18
//!   /recall body-migrate precedent committed at 706a642.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/rules` match arm at slash.rs:591-706 was *async* only
//! because it lived inside the async dispatch loop and emitted via
//! `tui_tx.send(..).await`. The underlying `RulesEngine` methods are
//! 100% sync. In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork + AGS-817 /memory + B18 /recall
//! precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used `s.strip_prefix("/rules").unwrap_or("").trim()`
//! on the full input string, so `/rules edit abc new text here` (four
//! tokens after the verb) was forwarded verbatim as the inner
//! `"edit abc new text here"`. The registry parser tokenizes on
//! whitespace, so `args` is `["edit", "abc", "new", "text", "here"]`.
//! To preserve the shipped single-string semantics while going through
//! the parser, the handler joins `args` with a single space then
//! `.trim()`s. This is byte-equivalent to the shipped behaviour for
//! all inputs: zero-arg `/rules` collapses to `""` (list branch),
//! `/rules list` collapses to `"list"`, `/rules edit <id> <text>`
//! collapses to `"edit <id> <text>"`, and `/rules remove <id>`
//! collapses to `"remove <id>"`. Same pattern as B18 /recall — see
//! `src/command/recall.rs:175-176`.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"List, edit, or remove behavioral
//!   rules"` — byte-identical to the `declare_handler!` stub at
//!   registry.rs:1336.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice).
//! - Emitted events preserve the shipped slash.rs:591-706 format
//!   strings BYTE-FOR-BYTE across all 13 branch outputs:
//!   * list empty → `TuiEvent::TextDelta("\nNo behavioral rules.\n")`.
//!   * list header → `format!("\n{} behavioral rules:\n\n",
//!     rules.len())` — count FIRST, then word "behavioral rules",
//!     trailing colon + blank line.
//!   * list per-rule → `format!("  [{id_short}] (score: {:.1}) {}\n",
//!     r.score, r.text)` — TWO spaces then bracket, ONE space between
//!     bracket and `(score: ...)`, `{:.1}` precision on score, ONE
//!     space, rule text, single trailing newline.
//!   * list failure → `TuiEvent::Error(format!("rules list failed:
//!     {e}"))`.
//!   * edit usage → `TuiEvent::Error("Usage: /rules edit <id> <new
//!     text>")`.
//!   * edit success → `TuiEvent::TextDelta(format!("\nRule updated:
//!     {new_text}\n"))`.
//!   * edit update failure → `TuiEvent::Error(format!("update_rule
//!     failed: {e}"))`.
//!   * edit lookup failure → `TuiEvent::Error(format!("rules lookup
//!     failed: {e}"))`.
//!   * edit/remove no-match → `TuiEvent::Error(format!("No rule
//!     matching ID prefix '{id_prefix}'"))` — literal single-quotes
//!     around `{id_prefix}`. Used by BOTH edit and remove.
//!   * remove success → `TuiEvent::TextDelta(format!("\nRule removed:
//!     {}\n", rule.text))` — note positional arg for rule.text.
//!   * remove delete failure → `TuiEvent::Error(format!("remove_rule
//!     failed: {e}"))`.
//!   * catch-all usage → `TuiEvent::Error("Usage: /rules [list | edit
//!     <id> <text> | remove <id>]")`.
//!   * id_short → `&r.id[..8.min(r.id.len())]` — byte slice with
//!     length cap (same as B18 /recall).
//!
//! # R5 — aliases = zero (shipped-wins)
//!
//! Shipped pre-B19: none (2-arg declare_handler! form at
//! registry.rs:1336). AGS-817 shipped-wins rule preserves zero
//! aliases. No aliases added. Matches /fork / /mcp / /context /
//! /hooks / /rename / /recall precedent.
//!
//! # R6 — memory field reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` is already
//! populated unconditionally by `build_command_context` per AGS-817
//! /memory (`context.rs:69` —
//! `memory: Some(Arc::clone(&slash_ctx.memory))`). This ticket REUSES
//! that exact field — there is no `rules_snapshot` type, no
//! context.rs match arm added, no new `build_command_context` wiring.
//! The test fixture helper (`make_rules_ctx`) mirrors the AGS-817
//! /memory and B18 /recall `make_ctx(memory)` shape.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `RulesHandler` (PATH A,
//! via the dispatcher at slash.rs:46) AND the legacy
//! `s if s == "/rules" || s.starts_with("/rules ")` match arm at
//! slash.rs:591-706 are live. Every `/rules` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE subsequent subagent run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.

use archon_consciousness::rules::RulesEngine;
use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/rules` command.
///
/// No aliases. Shipped pre-B19 stub carried none (2-arg
/// declare_handler! form at registry.rs:1336) and AGS-817 shipped-wins
/// rule preserves zero aliases. Matches /fork / /mcp / /context /
/// /hooks / /rename / /recall precedent.
pub(crate) struct RulesHandler;

impl RulesHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`RecallHandler::new`, `RenameHandler::new`, `DoctorHandler::new`,
    /// `UsageHandler::new`) even though the unit struct is constructible
    /// without it — the explicit constructor keeps the call site in
    /// registry.rs:1394 copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for RulesHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for RulesHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R3: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/rules").unwrap_or("").trim()`
        // for all inputs — zero-arg `/rules` collapses to "" (list
        // branch), `/rules list` collapses to "list", `/rules edit
        // <id> <text>` collapses to "edit <id> <text>", and `/rules
        // remove <id>` collapses to "remove <id>". Same pattern as
        // B18 /recall.
        let joined = args.join(" ");
        let args_str = joined.trim();

        // R6: require memory handle. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::memory` per
        // the AGS-817 /memory precedent (context.rs:69), so at the
        // real dispatch site this branch never fires. Test fixtures
        // that construct `CommandContext` directly with `memory: None`
        // will hit this branch and observe an Err — mirroring the
        // AGS-817 /memory and B18 /recall precedent.
        let memory_arc = ctx.memory.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "RulesHandler invoked without ctx.memory populated — \
                 build_command_context bug"
            )
        })?;
        let memory: &dyn MemoryTrait = memory_arc.as_ref();
        let engine = RulesEngine::new(memory);

        if args_str.is_empty() || args_str == "list" {
            // ── list branch ────────────────────────────────────────
            // Byte-for-byte preservation of shipped format strings at
            // slash.rs:595-618.
            match engine.get_rules_sorted() {
                Ok(rules) if rules.is_empty() => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                        "\nNo behavioral rules.\n".into(),
                    ));
                }
                Ok(rules) => {
                    let mut out =
                        format!("\n{} behavioral rules:\n\n", rules.len());
                    for r in &rules {
                        let id_short = &r.id[..8.min(r.id.len())];
                        out.push_str(&format!(
                            "  [{id_short}] (score: {:.1}) {}\n",
                            r.score, r.text
                        ));
                    }
                    let _ =
                        ctx.tui_tx.try_send(TuiEvent::TextDelta(out));
                }
                Err(e) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                        format!("rules list failed: {e}"),
                    ));
                }
            }
        } else if let Some(rest) = args_str.strip_prefix("edit ") {
            // ── edit branch ────────────────────────────────────────
            // `/rules edit <id> <new text>`. Byte-for-byte preservation
            // of shipped format strings at slash.rs:619-663.
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                    "Usage: /rules edit <id> <new text>".into(),
                ));
            } else {
                let id_prefix = parts[0];
                let new_text = parts[1];
                match engine.get_rules_sorted() {
                    Ok(rules) => {
                        if let Some(rule) = rules
                            .iter()
                            .find(|r| r.id.starts_with(id_prefix))
                        {
                            match engine.update_rule(&rule.id, new_text) {
                                Ok(()) => {
                                    let _ = ctx.tui_tx.try_send(
                                        TuiEvent::TextDelta(format!(
                                            "\nRule updated: {new_text}\n"
                                        )),
                                    );
                                }
                                Err(e) => {
                                    let _ = ctx.tui_tx.try_send(
                                        TuiEvent::Error(format!(
                                            "update_rule failed: {e}"
                                        )),
                                    );
                                }
                            }
                        } else {
                            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                                format!(
                                    "No rule matching ID prefix '{id_prefix}'"
                                ),
                            ));
                        }
                    }
                    Err(e) => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                            format!("rules lookup failed: {e}"),
                        ));
                    }
                }
            }
        } else if let Some(id_prefix) = args_str.strip_prefix("remove ") {
            // ── remove branch ──────────────────────────────────────
            // `/rules remove <id>`. Byte-for-byte preservation of
            // shipped format strings at slash.rs:664-697.
            let id_prefix = id_prefix.trim();
            match engine.get_rules_sorted() {
                Ok(rules) => {
                    if let Some(rule) = rules
                        .iter()
                        .find(|r| r.id.starts_with(id_prefix))
                    {
                        match engine.remove_rule(&rule.id) {
                            Ok(()) => {
                                let _ = ctx.tui_tx.try_send(
                                    TuiEvent::TextDelta(format!(
                                        "\nRule removed: {}\n",
                                        rule.text
                                    )),
                                );
                            }
                            Err(e) => {
                                let _ = ctx.tui_tx.try_send(
                                    TuiEvent::Error(format!(
                                        "remove_rule failed: {e}"
                                    )),
                                );
                            }
                        }
                    } else {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                            format!(
                                "No rule matching ID prefix '{id_prefix}'"
                            ),
                        ));
                    }
                }
                Err(e) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                        format!("rules lookup failed: {e}"),
                    ));
                }
            }
        } else {
            // ── catch-all usage ────────────────────────────────────
            // Byte-for-byte preservation of shipped format string at
            // slash.rs:699-703.
            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                "Usage: /rules [list | edit <id> <text> | remove <id>]"
                    .into(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1336.
        "List, edit, or remove behavioral rules"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice) and AGS-817
        // shipped-wins rule preserves zero aliases.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B19-RULES: tests for /rules slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use archon_consciousness::rules::RuleSource;
    use archon_memory::MemoryGraph;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::{CommandContext, RegistryBuilder};

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `memory` handle. Mirrors the AGS-817 /memory + B18
    /// /recall `make_ctx(memory)` fixture — DIRECT pattern, no
    /// snapshot, no effect slot.
    fn make_rules_ctx(
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
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    /// Build a real in-memory MemoryGraph wrapped in an Arc<dyn>. Using
    /// the real backend (rather than a StubMemory) exercises the full
    /// RulesEngine round-trip (search_memories → update_memory /
    /// delete_memory) in the same way the rules.rs unit tests do — see
    /// `crates/archon-consciousness/src/rules.rs:334-337`
    /// (`make_engine` helper) for the upstream pattern.
    fn make_graph() -> Arc<MemoryGraph> {
        Arc::new(
            MemoryGraph::in_memory()
                .expect("in-memory graph should succeed"),
        )
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1336. Any drift here means the stub and
    /// the new handler have diverged.
    #[test]
    fn rules_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            RulesHandler::new().description(),
            "List, edit, or remove behavioral rules"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice) and AGS-817
    /// shipped-wins preserves zero aliases.
    #[test]
    fn rules_handler_aliases_are_empty() {
        assert_eq!(RulesHandler::new().aliases(), &[] as &[&str]);
    }

    /// R6: when `memory` is None, execute returns Err whose message
    /// mentions both `memory` and `build_command_context` so the
    /// operator can trace the wiring bug. Mirrors the AGS-817 /memory
    /// and B18 /recall precedent.
    #[test]
    fn execute_without_memory_returns_err() {
        let (mut ctx, _rx) = make_rules_ctx(None);
        let h = RulesHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "RulesHandler::execute with None memory must return Err \
             (builder contract violation), got: {res:?}"
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

    /// List branch, empty graph: emits the byte-identical
    /// `"\nNo behavioral rules.\n"` TextDelta.
    #[test]
    fn execute_list_empty_emits_no_rules_textdelta() {
        let graph = make_graph();
        let memory: Arc<dyn MemoryTrait> = graph.clone();
        let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
        let h = RulesHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "list-empty must return Ok(()), got: {res:?}");

        let ev = rx.try_recv().expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nNo behavioral rules.\n",
                    "list-empty TextDelta must be byte-identical to \
                     the shipped slash.rs:598-600 literal"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with no-rules literal, \
                 got: {other:?}"
            ),
        }
    }

    /// List branch, non-empty graph: emits the byte-identical
    /// formatted-list TextDelta with the `.1`-precision score format
    /// per the shipped slash.rs:603-611 format loop.
    #[test]
    fn execute_list_non_empty_emits_formatted_textdelta() {
        let graph = make_graph();
        // Seed two rules with known text and scores. RulesEngine
        // sorts by score descending so rule_a (score 80.0) must come
        // before rule_b (score 30.0) in the output.
        {
            let engine = RulesEngine::new(graph.as_ref());
            let r_a = engine
                .add_rule("rule alpha", RuleSource::UserDefined)
                .expect("seed rule a");
            let r_b = engine
                .add_rule("rule beta", RuleSource::SystemDefault)
                .expect("seed rule b");
            graph
                .update_importance(&r_a.id, 80.0)
                .expect("set score a");
            graph
                .update_importance(&r_b.id, 30.0)
                .expect("set score b");
        }
        // Snapshot the sorted rules for byte-exact assertion.
        let sorted = RulesEngine::new(graph.as_ref())
            .get_rules_sorted()
            .expect("sorted list");
        assert_eq!(sorted.len(), 2, "seeded 2 rules");
        let expected = {
            let mut out =
                format!("\n{} behavioral rules:\n\n", sorted.len());
            for r in &sorted {
                let id_short = &r.id[..8.min(r.id.len())];
                out.push_str(&format!(
                    "  [{id_short}] (score: {:.1}) {}\n",
                    r.score, r.text
                ));
            }
            out
        };

        let memory: Arc<dyn MemoryTrait> = graph.clone();
        let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
        let h = RulesHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "list non-empty must return Ok(()), got: {res:?}"
        );

        let ev =
            rx.try_recv().expect("formatted TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, expected,
                    "list non-empty TextDelta must be byte-identical \
                     to the shipped slash.rs:603-611 format (count + \
                     word 'behavioral rules' + colon + blank line + \
                     two-space bracket + one-space '(score: {{:.1}})' + \
                     one-space text + single newline per rule)"
                );
                // Defence-in-depth: verify `.1` precision is honoured.
                assert!(
                    text.contains("(score: 80.0)"),
                    "expected '.1' precision '80.0' in output, got: {text}"
                );
                assert!(
                    text.contains("(score: 30.0)"),
                    "expected '.1' precision '30.0' in output, got: {text}"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with formatted list, \
                 got: {other:?}"
            ),
        }
    }

    /// Edit branch, success: seed a rule, resolve it by ID prefix,
    /// update its text, and verify the byte-identical
    /// `"\nRule updated: {new_text}\n"` TextDelta.
    #[test]
    fn execute_edit_success_emits_rule_updated() {
        let graph = make_graph();
        let rule_id: String = {
            let engine = RulesEngine::new(graph.as_ref());
            engine
                .add_rule("old text", RuleSource::UserDefined)
                .expect("seed rule")
                .id
        };
        let id_prefix: String = rule_id[..8.min(rule_id.len())].to_string();

        let memory: Arc<dyn MemoryTrait> = graph.clone();
        let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
        let h = RulesHandler::new();
        let args = vec![
            "edit".to_string(),
            id_prefix,
            "new".to_string(),
            "text".to_string(),
        ];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "edit success must return Ok(()), got: {res:?}"
        );

        let ev = rx.try_recv().expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nRule updated: new text\n",
                    "edit success TextDelta must be byte-identical to \
                     the shipped slash.rs:635-639 literal (note: the \
                     new_text here is 'new text' — two tokens joined \
                     via rest.splitn(2, ' ') which preserves the \
                     single whitespace)"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with rule-updated \
                 literal, got: {other:?}"
            ),
        }

        // Verify the rule text was actually updated in the graph.
        let rules = RulesEngine::new(graph.as_ref())
            .get_rules_sorted()
            .expect("resorted list");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].text, "new text",
            "edit branch must persist the new text via \
             RulesEngine::update_rule"
        );
    }

    /// Remove branch, success: seed a rule, resolve it by ID prefix,
    /// remove it, and verify the byte-identical
    /// `"\nRule removed: {rule.text}\n"` TextDelta.
    #[test]
    fn execute_remove_success_emits_rule_removed() {
        let graph = make_graph();
        let rule_id: String = {
            let engine = RulesEngine::new(graph.as_ref());
            engine
                .add_rule("doomed rule", RuleSource::UserDefined)
                .expect("seed rule")
                .id
        };
        let id_prefix: String = rule_id[..8.min(rule_id.len())].to_string();

        let memory: Arc<dyn MemoryTrait> = graph.clone();
        let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
        let h = RulesHandler::new();
        let args = vec!["remove".to_string(), id_prefix];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "remove success must return Ok(()), got: {res:?}"
        );

        let ev = rx.try_recv().expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nRule removed: doomed rule\n",
                    "remove success TextDelta must be byte-identical \
                     to the shipped slash.rs:671-676 literal (uses \
                     positional arg for rule.text)"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with rule-removed \
                 literal, got: {other:?}"
            ),
        }

        // Verify the rule was actually removed from the graph.
        let rules = RulesEngine::new(graph.as_ref())
            .get_rules_sorted()
            .expect("resorted list");
        assert!(
            rules.is_empty(),
            "remove branch must delete the rule via \
             RulesEngine::remove_rule"
        );
    }

    /// Dispatcher-integration (happy-path list-empty). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/rules` with
    /// `RulesHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/rules")` routes through the
    /// real alias+primary pipeline with a real in-memory
    /// MemoryGraph. Asserts the dispatcher's end-to-end wiring
    /// (parser → registry → handler.execute) delivers the byte-
    /// identical no-rules TextDelta.
    #[test]
    fn dispatcher_routes_slash_rules_with_memory_emits_textdelta() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("rules", Arc::new(RulesHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let graph = make_graph();
        let memory: Arc<dyn MemoryTrait> = graph.clone();
        let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
        let res = dispatcher.dispatch(&mut ctx, "/rules");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for the list-empty \
             path, got: {res:?}"
        );

        let ev = rx.try_recv().expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nNo behavioral rules.\n",
                    "dispatcher must deliver the byte-identical \
                     no-rules TextDelta through the full parser → \
                     registry → handler pipeline"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta with no-rules literal, \
                 got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/rules`, dispatches
    /// `"/rules"` with `memory: None`, and asserts that
    /// `Dispatcher::dispatch` surfaces the handler's Err. Mirrors
    /// the B18 /recall `dispatcher_routes_slash_recall_without_memory
    /// _returns_err` precedent.
    #[test]
    fn dispatcher_routes_slash_rules_without_memory_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("rules", Arc::new(RulesHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_rules_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/rules");
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
