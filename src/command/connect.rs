//! TASK-#214 SLASH-CONNECT — `/connect` slash-command handler.
//!
//! Subcommands:
//!   - `/connect` (no args)        → list configured servers + state
//!     (reuses the existing `mcp_snapshot` field on `CommandContext`;
//!     the snapshot-population arm in `src/command/context.rs` is
//!     widened from `Some("mcp")` to `Some("mcp") | Some("connect")`
//!     so the same async builder fires for either primary).
//!   - `/connect <name>`           → emit a TextDelta hint describing
//!     the canonical user path (edit `.mcp.json` + restart the
//!     session, or use the TUI MCP-manager overlay). The handler does
//!     NOT directly drive `lifecycle::connect_server` — see scope
//!     reconciliation below.
//!
//! # Spec/reality reconciliation
//!
//! The mission ticket said "dynamic MCP connect, wraps the
//! lifecycle::connect_server dispatcher". An initial implementation of
//! this ticket attempted to wrap
//! `McpServerManager::enable_server(name)` (the public entry that
//! dispatches through the private `lifecycle::connect::connect_server`
//! per-transport handler) via the `CommandEffect` slot pattern. That
//! attempt failed at compile time:
//!
//! ```text
//! error: higher-ranked lifetime error
//!    --> src/command/context.rs:364:5 ... apply_effect block
//!    = note: could not prove `Pin<Box<{async block ...}>>:
//!            CoerceUnsized<Pin<Box<dyn Future<Output = ()> + Send>>>`
//! ```
//!
//! The root cause: `enable_server`'s future composes through
//! `lifecycle::connect::connect_server`, which for the `ws`/`sse`
//! transports pulls in `tokio_tungstenite` + `rmcp::service` types
//! whose stream-conversion futures hold non-`Send` values across
//! `.await` points (verified by both the inline-await and
//! `tokio::spawn` variants compiling with the same E-class error —
//! `tokio::spawn` requires `Future + Send + 'static`, so detaching
//! does not bypass the bound; the bound IS the bound).
//!
//! Wrapping the call from a `Future + Send + 'a` apply_effect path
//! requires either:
//!   (a) making the upstream `lifecycle::connect_server` future
//!       `Send`-clean (subsystem refactor — touches archon-mcp's
//!       transport adapters and several rmcp/tungstenite call sites),
//!       or
//!   (b) standing up a session-wide `LocalSet` so the connect work
//!       can run via `tokio::task::spawn_local` — also cross-cutting,
//!       requires changes to the session bootstrap in `src/session.rs`
//!       and a non-trivial `LocalSet`-aware run loop.
//!
//! Both options exceed the wrapper-scope ceiling for this ticket.
//! Resolution per the mission "Spec-reality drift → reconcile, adapt
//! scope, document in commit body" rule:
//!
//!   - Ship `/connect` as a LIST + HINT command. The no-args path
//!     gives the user immediate value (current server states, tool
//!     counts, whether anything is configured at all). The with-name
//!     path emits a TextDelta pointing at the canonical user path
//!     (`.mcp.json` config + session restart, or the TUI MCP-manager
//!     overlay) and explains WHY the dynamic connect is currently
//!     deferred. Honest scope reduction; no silent failure.
//!   - Defer the actual `enable_server` wrapping to a follow-up
//!     ticket that resolves either (a) or (b) above.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/connect` handler — MCP server list + connect-hint.
pub(crate) struct ConnectHandler;

impl CommandHandler for ConnectHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // Args may arrive as `["foo"]`, `[]`, or `["", ""]` (the parser
        // strips leading whitespace but tolerates empty positionals).
        // Treat any empty / whitespace-only first arg as "no args".
        let name = args
            .iter()
            .find(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());

        let body = match name {
            None => render_list(ctx.mcp_snapshot.as_ref()),
            Some(server_name) => render_connect_hint(&server_name),
        };
        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "List configured MCP servers (with name: show how to connect)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Render the no-args list view from the supplied `mcp_snapshot`.
/// Pulled out so unit tests can exercise both populated and `None`
/// branches without standing up a full builder.
fn render_list(snapshot: Option<&crate::command::mcp::McpSnapshot>) -> String {
    let mut out = String::with_capacity(512);
    out.push('\n');
    out.push_str("/connect — Configured MCP servers\n");
    out.push_str("─────────────────────────────────\n");

    match snapshot {
        None => {
            out.push_str(
                "MCP server list unavailable in this context — the\n\
                 mcp_snapshot field was not populated by the dispatch\n\
                 builder. This is a wiring regression; please report.\n\n",
            );
        }
        Some(snap) if snap.entries.is_empty() => {
            out.push_str(
                "No MCP servers are configured. To add one, edit\n\
                 .mcp.json (project) or ~/.config/archon/.mcp.json\n\
                 (global) and restart the session.\n\n",
            );
        }
        Some(snap) => {
            out.push_str(&format!("Servers ({} total):\n", snap.entries.len()));
            out.push_str(&format!("  {:<24}  {:<10}  tools\n", "name", "state"));
            out.push_str(&format!(
                "  {}  {}  -----\n",
                "-".repeat(24),
                "-".repeat(10)
            ));
            for entry in &snap.entries {
                out.push_str(&format!(
                    "  {:<24}  {:<10}  {}\n",
                    truncate_chars(&entry.name, 24),
                    entry.state,
                    entry.tool_count,
                ));
            }
            out.push('\n');
        }
    }

    out.push_str("Usage:  /connect <name>          show how to connect/restart that server\n");
    out.push_str("        /connect                 list configured servers (this view)\n");
    out
}

/// Render the with-name "how to connect" hint. The actual dynamic
/// connect is deferred (see module rustdoc); this body is the honest
/// pointer to the canonical user path.
fn render_connect_hint(name: &str) -> String {
    format!(
        "\n\
         /connect — Connect to MCP server `{name}`\n\
         ─────────────────────────────────────────\n\
         Dynamic in-session MCP connect is currently deferred (see\n\
         `src/command/connect.rs` module rustdoc — the underlying\n\
         lifecycle::connect_server future has Send-bound issues that\n\
         need an upstream fix or a session-wide LocalSet to wrap from\n\
         the slash-command surface).\n\
         \n\
         To connect `{name}` today:\n  \
         1. Make sure `{name}` is defined in .mcp.json (project) or\n     \
            ~/.config/archon/.mcp.json (global) with the right\n     \
            transport / command / args / url for your environment.\n  \
         2. If it is already defined but `disabled: true`, flip that\n     \
            to `false`.\n  \
         3. Restart the session, OR open the TUI MCP-manager overlay\n     \
            and use its connect/restart action (which DOES run from\n     \
            an async-friendly context).\n\
         \n\
         Run `/connect` (no args) for the current configured-server\n\
         list and per-server state.\n",
        name = name
    )
}

/// Truncate to at most `max` Unicode characters, appending `…` when
/// shortened. Char-aware — never panics on multi-byte input.
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn drain_one(rx: &mut tokio::sync::mpsc::UnboundedReceiver<TuiEvent>) -> TuiEvent {
        let evs = drain_tui_events(rx);
        assert_eq!(evs.len(), 1, "expected exactly one event; got {:?}", evs);
        evs.into_iter().next().unwrap()
    }

    fn snapshot_with_entries(
        entries: Vec<archon_tui::app::McpServerEntry>,
    ) -> crate::command::mcp::McpSnapshot {
        crate::command::mcp::McpSnapshot { entries }
    }

    fn entry(name: &str, state: &str, tool_count: usize) -> archon_tui::app::McpServerEntry {
        archon_tui::app::McpServerEntry {
            name: name.to_string(),
            state: state.to_string(),
            tool_count,
            disabled: false,
            tools: Vec::new(),
        }
    }

    #[test]
    fn no_args_with_empty_snapshot_emits_no_servers_message() {
        let handler = ConnectHandler;
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_mcp_snapshot(snapshot_with_entries(Vec::new()))
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("No MCP servers are configured"));
        assert!(body.contains(".mcp.json"));
    }

    #[test]
    fn no_args_with_populated_snapshot_lists_servers() {
        let handler = ConnectHandler;
        let snap = snapshot_with_entries(vec![
            entry("filesystem", "ready", 12),
            entry("github", "stopped", 0),
        ]);
        let (mut ctx, mut rx) = CtxBuilder::new().with_mcp_snapshot(snap).build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Servers (2 total)"));
        assert!(body.contains("filesystem"));
        assert!(body.contains("github"));
        assert!(body.contains("ready"));
        assert!(body.contains("stopped"));
    }

    #[test]
    fn no_args_without_snapshot_emits_wiring_regression_message() {
        let handler = ConnectHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("MCP server list unavailable"));
        assert!(body.contains("wiring regression"));
    }

    #[test]
    fn with_name_arg_emits_connect_hint() {
        let handler = ConnectHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[String::from("filesystem")])
            .unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        // Hint body should mention the server name + canonical
        // user-path (.mcp.json + restart, or TUI overlay).
        assert!(body.contains("Connect to MCP server `filesystem`"));
        assert!(body.contains(".mcp.json"));
        assert!(body.contains("Restart the session"));
        assert!(body.contains("TUI MCP-manager overlay"));
        // No effect should be stashed — scope reduction is honest.
        assert!(
            ctx.pending_effect.is_none(),
            "no effect should be stashed in scope-reduced /connect"
        );
    }

    #[test]
    fn with_empty_string_arg_treated_as_no_args() {
        // Defensive: parser may emit `[""]` for `/connect  ` with
        // trailing spaces. Empty/whitespace-only first arg falls back
        // to the list path.
        let handler = ConnectHandler;
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_mcp_snapshot(snapshot_with_entries(Vec::new()))
            .build();
        handler.execute(&mut ctx, &[String::from("   ")]).unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("No MCP servers are configured"));
        assert!(ctx.pending_effect.is_none());
    }

    #[test]
    fn extra_args_after_name_are_ignored() {
        // Defensive: `/connect foo bar baz` treats "foo" as the name
        // and ignores trailing positional args.
        let handler = ConnectHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
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
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Connect to MCP server `foo`"));
        // "bar" and "baz" should NOT appear as server names.
        assert!(!body.contains("`bar`"));
    }

    #[test]
    fn truncate_chars_unicode_safe() {
        let truncated = truncate_chars("αβγδεζηθικ", 5);
        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn description_and_aliases() {
        let h = ConnectHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn connect_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("connect") must return Some;
        // executing with arg "smoke-server" emits the connect hint;
        // executing with no args + an empty mcp_snapshot emits the
        // no-servers message.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("connect")
            .expect("connect must be registered in default_registry()");

        // With-arg path:
        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[String::from("smoke-server")])
            .unwrap();
        let body = match drain_one(&mut rx) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Connect to MCP server `smoke-server`"));

        // No-args path with empty snapshot:
        let (mut ctx2, mut rx2) = CtxBuilder::new()
            .with_mcp_snapshot(snapshot_with_entries(Vec::new()))
            .build();
        handler.execute(&mut ctx2, &[]).unwrap();
        let body2 = match drain_one(&mut rx2) {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body2.contains("/connect"));
        assert!(body2.contains("No MCP servers are configured"));
    }
}
