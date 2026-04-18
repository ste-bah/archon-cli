//! TASK-AGS-811: /mcp slash-command handler (body-migrate target,
//! Option C, SNAPSHOT-ONLY pattern reuse).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:662-691`. Third SNAPSHOT body-migrate under
//! Option C (after AGS-807 /status and AGS-809 /cost).
//!
//! # Why SNAPSHOT-ONLY (no effect slot)?
//!
//! The shipped /mcp body is READ-ONLY — it calls
//! `ctx.mcp_manager.get_server_info().await` once, then optionally
//! `ctx.mcp_manager.list_tools_for(&name).await` for each Ready server,
//! and emits the resulting list via
//! `TuiEvent::ShowMcpManager(Vec<McpServerEntry>)`. There are no writes
//! back to `SlashCommandContext` state — every mutation lives inside the
//! `McpServerManager`'s own `Arc<RwLock<..>>` and is owned by
//! `archon_mcp::lifecycle`.
//!
//! Because `CommandHandler::execute` is SYNC (Q1=A invariant), the
//! `.await` calls on `McpServerManager` are not legal inside `execute`.
//! Solution (same snapshot pattern as AGS-807 `/status` and AGS-809
//! `/cost`): the dispatch site at `slash.rs` (via
//! `build_command_context`) builds an [`McpSnapshot`] by awaiting
//! `get_server_info` + the N per-server `list_tools_for` calls BEFORE
//! calling `Dispatcher::dispatch`, pre-computing every `McpServerEntry`
//! inside the builder, and threads the owned `Vec<McpServerEntry>`
//! through [`CommandContext`] so the sync handler consumes without
//! holding any async-mutex guard.
//!
//! /mcp is READ-ONLY — there is no `CommandEffect` variant for this
//! ticket. Subcommands (`connect` / `disconnect` / `reload`) specified
//! in `TASK-AGS-811.md` are SCOPE-HELD: shipped only exposes LIST, and
//! the body-migrate pattern is "shipped wins" on drift-reconcile. The
//! first ticket that actually needs those write paths will add them
//! per AGS-822 Rule 5 (add the dep when the migrating handler body
//! actually needs it).
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted value is faithful to the deleted slash.rs:662-691 body:
//! - `state_str` mapping: "disabled" | "ready" | "starting" | "crashed"
//!   | "stopped" — EXACT. `Starting` and `Restarting` coalesce to the
//!   single "starting" label (shipped behaviour).
//! - `McpServerEntry` fields `{ name, state, tool_count, disabled,
//!   tools }` populated EXACTLY.
//! - `tools` list: empty `Vec::new()` unless `state_str == "ready"`,
//!   matching the shipped conditional on the Ready branch.
//! - Event variant: `TuiEvent::ShowMcpManager(entries)` — unchanged.
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching
//! AGS-806/807/808/809/810 precedent. /mcp is best-effort UI —
//! dropping a LIST event under 16-cap channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-AGS-811: no aliases (empty slice). Spec
//! `TASK-AGS-811.md` does not list any new aliases either. `aliases()`
//! returns `&[]`. No drift to reconcile.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of every value the /mcp body reads from shared state.
/// Built at the dispatch site (where `.await` is allowed) and threaded
/// through [`CommandContext`] so the sync handler can consume without
/// holding any async-mutex guard on `McpServerManager`.
///
/// The inner `Vec<McpServerEntry>` is fully owned — each entry's `name`
/// / `state` / `tools` are `String` / `Vec<String>`. No `Arc`, no
/// `Mutex`, no borrows. Pre-computing the entries list inside the
/// builder means the handler is zero-`.await` and pays zero lock
/// traffic at dispatch time.
#[derive(Debug, Clone)]
pub(crate) struct McpSnapshot {
    /// Pre-captured list of MCP server entries, ready to hand off to
    /// `TuiEvent::ShowMcpManager`. Every field on every entry is
    /// populated by the shipped state-string mapping + the ready-gated
    /// `list_tools_for` call, preserved byte-for-byte from
    /// `src/command/slash.rs:662-691`.
    pub(crate) entries: Vec<archon_tui::app::McpServerEntry>,
}

/// Build an [`McpSnapshot`] by awaiting `McpServerManager::get_server_info`
/// and (for each Ready server) `McpServerManager::list_tools_for` in the
/// SAME order as the shipped `/mcp` body at
/// `src/command/slash.rs:662-691`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/mcp`. All other commands leave `mcp_snapshot = None`
/// to avoid unnecessary lock traffic on `McpServerManager`.
///
/// The state-string mapping and the `state_str == "ready"` conditional
/// are preserved exactly — future readers should consult the shipped
/// body's diff-history if the mapping needs to change.
pub(crate) async fn build_mcp_snapshot(
    slash_ctx: &SlashCommandContext,
) -> McpSnapshot {
    // Single `get_server_info` call, matching the shipped one-shot read.
    // Returns `Vec<(String, ServerState, bool)>` — (name, state,
    // is_disabled).
    let info = slash_ctx.mcp_manager.get_server_info().await;

    let mut entries: Vec<archon_tui::app::McpServerEntry> =
        Vec::with_capacity(info.len());

    for (name, state, disabled) in info {
        // State-string mapping — byte-for-byte from shipped body.
        // Disabled servers short-circuit to the "disabled" label
        // regardless of their underlying ServerState. Starting and
        // Restarting coalesce to a single "starting" user-visible
        // label. Unknown variants are not possible at the enum level.
        let state_str = if disabled {
            "disabled"
        } else {
            match state {
                archon_mcp::types::ServerState::Ready => "ready",
                archon_mcp::types::ServerState::Starting
                | archon_mcp::types::ServerState::Restarting => "starting",
                archon_mcp::types::ServerState::Crashed => "crashed",
                archon_mcp::types::ServerState::Stopped => "stopped",
            }
        };

        // Tools are only listed for Ready servers — matches the
        // shipped conditional. Every other state_str yields an empty
        // Vec to avoid probing a non-ready client.
        let tools = if state_str == "ready" {
            slash_ctx.mcp_manager.list_tools_for(&name).await
        } else {
            Vec::new()
        };

        entries.push(archon_tui::app::McpServerEntry {
            name: name.clone(),
            state: state_str.to_string(),
            tool_count: tools.len(),
            disabled,
            tools,
        });
    }

    McpSnapshot { entries }
    // No guards to drop — `McpServerManager` methods each take/release
    // their own `RwLock` guard internally. The snapshot owns the
    // resulting owned values.
}

/// Zero-sized handler registered as the primary `/mcp` command.
///
/// Aliases: none (empty slice). Spec lists no aliases and shipped stub
/// at `registry.rs:467` registered none either.
pub(crate) struct McpHandler;

impl CommandHandler for McpHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Defensive: build_command_context is responsible for populating
        // mcp_snapshot when the primary resolves to /mcp. A None here
        // indicates a wiring regression (e.g. the builder was bypassed
        // or the alias map drifted), not a user-facing error — but we
        // surface it as an anyhow::Error so the bug is loud rather than
        // silent. Mirrors AGS-807/808/809 defensive pattern.
        let snap = ctx.mcp_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "McpHandler invoked without mcp_snapshot populated \
                 — build_command_context bug"
            )
        })?;

        // Clone the entries list into the event payload. Cheap: the
        // snapshot owns the entries and cloning a `Vec<McpServerEntry>`
        // is an O(N) String clone per entry; N is tiny (single-digit
        // server counts in practice).
        let _ = ctx
            .tui_tx
            .try_send(TuiEvent::ShowMcpManager(snap.entries.clone()));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show MCP server status"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped pre-AGS-811: no aliases. Spec lists none either.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-811: tests for /mcp slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::{McpServerEntry, TuiEvent};
    use tokio::sync::mpsc;

    /// Minimal fixture snapshot with one Ready server and one Disabled
    /// server. Values are chosen so the round-trip through the event
    /// keeps every field visible in assertion output.
    fn fixture_snapshot() -> McpSnapshot {
        McpSnapshot {
            entries: vec![
                McpServerEntry {
                    name: "memorygraph".to_string(),
                    state: "ready".to_string(),
                    tool_count: 2,
                    disabled: false,
                    tools: vec![
                        "mcp__memorygraph__store".to_string(),
                        "mcp__memorygraph__recall".to_string(),
                    ],
                },
                McpServerEntry {
                    name: "offline-server".to_string(),
                    state: "disabled".to_string(),
                    tool_count: 0,
                    disabled: true,
                    tools: Vec::new(),
                },
            ],
        }
    }

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied optional mcp snapshot. Tests exercising the defensive
    /// None branch pass `None`; tests exercising the happy path pass
    /// `Some(McpSnapshot { .. })`.
    fn make_ctx(
        snapshot: Option<McpSnapshot>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: snapshot,
                // TASK-AGS-814: /mcp tests never exercise /context paths — None.
                context_snapshot: None,
                // TASK-AGS-815: /mcp tests never exercise /fork paths — None.
                session_id: None,
                // TASK-AGS-817: /mcp tests never exercise /memory paths — None.
                memory: None,
                pending_effect: None,
            },
            rx,
        )
    }

    #[test]
    fn mcp_handler_description_matches() {
        let h = McpHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("mcp") || desc.contains("server"),
            "McpHandler description should reference 'mcp' or 'server', \
             got: {}",
            h.description()
        );
    }

    #[test]
    fn mcp_handler_aliases_are_empty() {
        let h = McpHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "McpHandler must register NO aliases — neither shipped stub \
             nor AGS-811 spec lists any"
        );
    }

    #[test]
    fn mcp_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = McpHandler;
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "McpHandler::execute must return Err when mcp_snapshot is \
             None (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("mcp_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn mcp_handler_execute_with_snapshot_emits_show_mcp_manager() {
        let (mut ctx, mut rx) = make_ctx(Some(fixture_snapshot()));
        let h = McpHandler;
        h.execute(&mut ctx, &[])
            .expect("McpHandler::execute must return Ok with snapshot populated");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::ShowMcpManager(entries) => {
                assert_eq!(
                    entries.len(),
                    2,
                    "ShowMcpManager must carry both fixture entries"
                );
                // Ready server — byte-for-byte fields preserved.
                let ready = &entries[0];
                assert_eq!(ready.name, "memorygraph");
                assert_eq!(ready.state, "ready");
                assert_eq!(ready.tool_count, 2);
                assert!(!ready.disabled);
                assert_eq!(ready.tools.len(), 2);
                // Disabled server — state_str short-circuits to
                // "disabled" regardless of underlying ServerState.
                let disabled = &entries[1];
                assert_eq!(disabled.name, "offline-server");
                assert_eq!(disabled.state, "disabled");
                assert_eq!(disabled.tool_count, 0);
                assert!(disabled.disabled);
                assert!(disabled.tools.is_empty());
            }
            other => panic!(
                "expected TuiEvent::ShowMcpManager, got {other:?}"
            ),
        }
    }

    #[test]
    fn mcp_snapshot_round_trip_via_clone() {
        // Sanity: McpSnapshot derives Debug + Clone and that cloning
        // preserves every field. Required because the type is inserted
        // into Option<McpSnapshot> in CommandContext and read back by
        // the handler (no Copy on Vec<McpServerEntry>).
        let snap = fixture_snapshot();
        let cloned = snap.clone();
        assert_eq!(snap.entries.len(), cloned.entries.len());
        for (a, b) in snap.entries.iter().zip(cloned.entries.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.state, b.state);
            assert_eq!(a.tool_count, b.tool_count);
            assert_eq!(a.disabled, b.disabled);
            assert_eq!(a.tools, b.tools);
        }
        // Debug impl must not panic.
        let _ = format!("{snap:?}");
    }
}
