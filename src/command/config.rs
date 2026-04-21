//! `/config` slash command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.
//!
//! # TASK-AGS-POST-6-NO-STUB: ConfigHandler (THIN-WRAPPER)
//!
//! Historically `/config` was wired into the dispatcher via a
//! `declare_handler!(ConfigHandler, "Show or update Archon
//! configuration", &["settings", "prefs"])` macro invocation in
//! `src/command/registry.rs:1346-1350`. The real async work lives
//! UPSTREAM in `src/command/slash.rs:247` which intercepts `/config`
//! before the dispatcher runs (it needs async file/tool access that
//! the sync `CommandHandler::execute` signature cannot `.await`), so
//! the registered handler was, and remains, a no-op wrapper.
//!
//! POST-6-NO-STUB eliminates the `declare_handler!` macro entirely.
//! To do so, the macro-generated ConfigHandler struct + impl is moved
//! into this file as a THIN-WRAPPER (same pattern as
//! `src/command/compact.rs` and `src/command/clear.rs`). The body is
//! byte-identical to the shipped macro stub: `execute` returns
//! `Ok(())` WITHOUT emitting any `TuiEvent`. The description literal
//! (`"Show or update Archon configuration"`) and aliases
//! (`&["settings", "prefs"]`) are preserved verbatim per AGS-817
//! shipped-wins drift-reconcile.
//!
//! The real-work async delegate `handle_config_command` below is
//! unchanged by POST-6-NO-STUB and continues to be the upstream
//! intercept target at `src/command/slash.rs:247`.

use crate::cli_args::Cli;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

/// Handle `/config` commands: list, get, set.
pub async fn handle_config_command(
    input: &str,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &SlashCommandContext,
) {
    let args: Vec<&str> = input
        .strip_prefix("/config")
        .unwrap_or_default()
        .trim()
        .splitn(2, ' ')
        .collect();
    let key = args.first().map(|s| s.trim()).unwrap_or("");
    let value = args.get(1).map(|s| s.trim()).unwrap_or("");

    if key == "sources" {
        let output = archon_core::config_source::format_sources(&ctx.config_sources);
        if output.is_empty() {
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nNo config sources tracked.\n".into()))
                .await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\nConfig sources:\n{output}")))
                .await;
        }
        return;
    }

    if key.is_empty() {
        // List all config keys with current values
        let keys = archon_tools::config_tool::all_keys();
        let mut lines = String::from("\nRuntime configuration:\n");
        for k in &keys {
            let val =
                archon_tools::config_tool::get_config_value(k).unwrap_or_else(|| "(unknown)".into());
            lines.push_str(&format!("  {k} = {val}\n"));
        }
        let _ = tui_tx.send(TuiEvent::TextDelta(lines)).await;
    } else if value.is_empty() {
        // Get a single key
        match archon_tools::config_tool::get_config_value(key) {
            Some(val) => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{key} = {val}\n")))
                    .await;
            }
            None => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Unknown config key: {key}")))
                    .await;
            }
        }
    } else {
        // Set key=value via the ConfigTool
        use archon_tools::tool::{AgentMode, ToolContext};
        let tool = archon_tools::config_tool::ConfigTool;
        let tool_ctx = ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: String::new(),
            mode: AgentMode::Normal,
            extra_dirs: Vec::new(),
            ..Default::default()
        };
        let result = archon_tools::tool::Tool::execute(
            &tool,
            serde_json::json!({ "action": "set", "key": key, "value": value }),
            &tool_ctx,
        )
        .await;
        if result.is_error {
            let _ = tui_tx.send(TuiEvent::Error(result.content)).await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\n{}\n", result.content)))
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-NO-STUB: ConfigHandler (THIN-WRAPPER macro-eliminator)
// ---------------------------------------------------------------------------

/// Zero-sized handler registered as the primary `/config` command.
///
/// Aliases: `["settings", "prefs"]` — PRESERVED from the shipped
/// `declare_handler!(ConfigHandler, ...)` macro invocation at
/// `src/command/registry.rs:1346-1350` (shipped-wins drift-reconcile
/// per AGS-813).
///
/// Under normal operation this handler is UNREACHABLE because
/// `src/command/slash.rs:247` intercepts `/config` before the
/// dispatcher runs and invokes the async `handle_config_command`
/// delegate in this same module. If `execute` fires anyway the
/// behavior is the shipped no-op `Ok(())` — byte-identical to the
/// pre-POST-6-NO-STUB `declare_handler!` macro body.
pub(crate) struct ConfigHandler;

impl ConfigHandler {
    /// Construct a fresh `ConfigHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for ConfigHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for ConfigHandler {
    fn execute(
        &self,
        _ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // THIN-WRAPPER no-op. Byte-identical to the shipped
        // `declare_handler!` macro body: return `Ok(())` WITHOUT
        // emitting any TuiEvent. Real async work happens UPSTREAM at
        // `src/command/slash.rs:247` via `handle_config_command`.
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1346-1350 (shipped-wins drift-reconcile).
        "Show or update Archon configuration"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub used the three-arg declare_handler! form with
        // `&["settings", "prefs"]`. Preserved per AGS-813 alias-only
        // drift-reconcile + AGS-817 shipped-wins precedent.
        &["settings", "prefs"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-NO-STUB: tests for /config ConfigHandler THIN-WRAPPER
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    /// Build a minimal `CommandContext` with a freshly-created channel.
    /// /config is a THIN-WRAPPER handler — no snapshot, no effect slot,
    /// no extra context field — so every optional field stays `None`.
    /// Mirrors the `make_ctx` fixtures in compact.rs / clear.rs /
    /// cancel.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn config_handler_description_byte_identical_to_shipped() {
        let h = ConfigHandler::new();
        assert_eq!(
            h.description(),
            "Show or update Archon configuration",
            "ConfigHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn config_handler_aliases_match_shipped() {
        let h = ConfigHandler::new();
        assert_eq!(
            h.aliases(),
            &["settings", "prefs"],
            "ConfigHandler aliases must preserve ['settings', 'prefs'] \
             from the shipped declare_handler! stub (AGS-813 alias-only \
             drift-reconcile + AGS-817 shipped-wins precedent)"
        );
    }

    #[test]
    fn config_handler_execute_returns_ok_without_emission() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ConfigHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ConfigHandler::execute must return Ok(()), got: {res:?}"
        );
        // No TuiEvent must be emitted — THIN-WRAPPER is byte-identical
        // to the shipped declare_handler! no-op stub. Real async work
        // happens UPSTREAM at slash.rs:247.
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "ConfigHandler::execute must NOT emit any TuiEvent, \
                 got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_config_returns_ok_without_emission() {
        // Narrow `RegistryBuilder::new()` (not `default_registry`) so
        // this test exercises ONLY the ConfigHandler wiring — no other
        // handlers are registered. Asserts the real Dispatcher routes
        // `/config` to `ConfigHandler::execute` and emits no event.
        let mut b = RegistryBuilder::new();
        b.insert_primary("config", Arc::new(ConfigHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/config");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/config\") must return Ok(()), \
             got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route to ConfigHandler must NOT emit any \
                 TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_alias_settings_returns_ok_without_emission() {
        // Verify the `settings` alias resolves to ConfigHandler through
        // the Registry's alias map (TASK-AGS-802). This pins the
        // shipped-wins alias-preservation invariant: `/settings` must
        // reach ConfigHandler::execute and return Ok(()) with no
        // emission — byte-identical to `/config`.
        let mut b = RegistryBuilder::new();
        b.insert_primary("config", Arc::new(ConfigHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/settings");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/settings\") must return Ok(()) \
             via the ConfigHandler alias, got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route via /settings alias to ConfigHandler \
                 must NOT emit any TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }
}
