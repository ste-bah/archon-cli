//! TASK-AGS-POST-6-BODIES-B20-RELOAD: /reload slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1280` and the legacy match arm at
//! `src/command/slash.rs:357-387`.
//!
//! # R1 — pattern = DIRECT (NOT EFFECT-SLOT as the B20 task tag suggests)
//!
//! The recon for this ticket proved DIRECT is the correct pattern. The
//! shipped `/reload` body calls a single pure-sync entry point —
//! `archon_core::config_watcher::force_reload(config_paths: &[PathBuf],
//! current: &ArchonConfig) -> Result<(ArchonConfig, Vec<String>),
//! ConfigError>` at `crates/archon-core/src/config_watcher.rs:160`.
//! No `.await` in the arm, no `tokio::sync::Mutex` guards on the
//! read/write path, and no writes back to `SlashCommandContext` state.
//! Consequently:
//!
//! - NO `ReloadSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits `TuiEvent`s — matches AGS-815 /fork and B17 /rename
//!   DIRECT precedent).
//! - A NEW `CommandContext::config_path: Option<PathBuf>` field is
//!   added and populated UNCONDITIONALLY by `build_command_context`
//!   (mirrors the AGS-815 `session_id` / AGS-817 `memory` cross-cutting
//!   precedent — not the per-primary SNAPSHOT gating pattern). `PathBuf`
//!   clone is cheap; every handler observes this field for free without
//!   a per-command builder match arm.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/reload` match arm at slash.rs:358-386 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying
//! `config_watcher::force_reload` call is 100% sync (no `async fn`,
//! no `.await` in its body). In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches the B17 /rename precedent exactly.
//!
//! # R3 — args ignored (shipped silent-ignore behaviour preserved)
//!
//! The shipped match arm took no args — it matched on the literal
//! `"/reload"` string, so trailing tokens like `/reload foo` never
//! reached this branch (they fell through the bare-literal match arm
//! and were handled by the skill-fallback or parse error path). Under
//! the new registry dispatcher the parser tokenizes `/reload foo` into
//! `name = "reload"` + `args = ["foo"]` and routes to
//! `ReloadHandler::execute`. To preserve the shipped silent-ignore
//! behaviour the handler simply ignores `args` — does NOT emit an
//! error for unexpected arguments. Byte-equivalent to shipped for
//! every possible input that used to reach the arm (`/reload` alone),
//! and strictly wider / permissive for inputs that didn't.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Reload configuration from disk"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1280.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and spec lists none.
//! - Emitted events preserve the shipped slash.rs:358-386 format
//!   strings byte-for-byte:
//!   * No-change branch → `TuiEvent::TextDelta(
//!     "\nConfig reloaded. No changes detected.\n")`.
//!   * With-change branch → `TuiEvent::TextDelta(format!(
//!     "\nConfig reloaded. Changed: {}\n", changed.join(", ")))`.
//!   * Err branch → `TuiEvent::Error(format!(
//!     "Config reload failed: {e}"))`.
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B20: none (2-arg declare_handler! form at
//! registry.rs:1280). Spec lists none. No aliases added. Matches
//! /fork / /rename / /mcp / /context / /hooks precedent.
//!
//! # R6 — config_path unconditional-populate (AGS-815-style)
//!
//! Unlike the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which
//! gate their populate step on the primary name, this ticket extends
//! `CommandContext` with `config_path: Option<PathBuf>` populated
//! UNCONDITIONALLY in `build_command_context` — mirroring the AGS-815
//! `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
//! B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
//! `skill_registry`, and B13-GARDEN `garden_config` DIRECT cross-cutting
//! precedent. `PathBuf` clone per dispatch is cheap (one Vec<u8> alloc
//! on the heap); future DIRECT handlers that need the config path
//! inherit this field for free without a per-command builder match
//! arm.
//!
//! The production builder always populates
//! `Some(slash_ctx.config_path.clone())`. `None` is the sentinel
//! reserved for test fixtures that construct `CommandContext` directly
//! without standing up a full `SlashCommandContext`; in those tests
//! the handler observes `None` and returns an Err-with-message
//! describing the missing-config_path condition rather than panicking.
//! Mirrors the AGS-815 `fork_handler_execute_without_session_id_returns_err`
//! pattern.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `ReloadHandler` (PATH A,
//! via the dispatcher at slash.rs:46) AND the legacy `"/reload" =>`
//! match arm at slash.rs:357-387 are live. Every `/reload` invocation
//! therefore fires twice — once via the handler and once via the
//! legacy arm. This is the Stage-6 body-migrate protocol: Gate 5
//! deletes the legacy match arm in a SEPARATE subsequent subagent run
//! (NOT this subagent's responsibility). Do NOT touch slash.rs in
//! this ticket.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/reload` command.
///
/// No aliases. Shipped pre-B20 stub carried none (2-arg
/// declare_handler! form at registry.rs:1280); spec lists none.
/// Matches /fork / /rename / /mcp / /context / /hooks precedent.
pub(crate) struct ReloadHandler;

impl ReloadHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`,
    /// `RenameHandler::new`, `RecallHandler::new`, `RulesHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for ReloadHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for ReloadHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // R6: require config_path. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::config_path`
        // per the AGS-815 session_id / AGS-817 memory cross-cutting
        // precedent, so at the real dispatch site this branch never
        // fires. Test fixtures that construct `CommandContext`
        // directly with `config_path: None` will hit this branch and
        // observe an Err — mirroring the AGS-815
        // `fork_handler_execute_without_session_id_returns_err`
        // pattern.
        let config_path = ctx.config_path.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "ReloadHandler invoked without ctx.config_path populated — \
                 build_command_context bug"
            )
        })?;

        // R2: sync call to `archon_core::config_watcher::force_reload`.
        // `std::slice::from_ref(config_path)` builds a one-element
        // `&[PathBuf]` without allocating. `ArchonConfig::default()`
        // matches the shipped arm's `current` argument exactly
        // (slash.rs:362) — the shipped body never threaded the live
        // config through to force_reload, so the diff is always
        // against defaults. Preserves shipped behaviour byte-for-byte
        // (the `_new_cfg` return is discarded in both old and new
        // paths — force_reload is called for the `changed` side-info
        // only; no write-back to shared config state happens here).
        match archon_core::config_watcher::force_reload(
            std::slice::from_ref(config_path),
            &archon_core::config::ArchonConfig::default(),
        ) {
            Ok((_new_cfg, changed)) if changed.is_empty() => {
                // R4 no-change branch — byte-for-byte preservation of
                // shipped format string at slash.rs:368.
                ctx.emit(TuiEvent::TextDelta(
                    "\nConfig reloaded. No changes detected.\n".into(),
                ));
            }
            Ok((_new_cfg, changed)) => {
                // R4 with-change branch — byte-for-byte preservation of
                // shipped format string at slash.rs:374-376.
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nConfig reloaded. Changed: {}\n",
                    changed.join(", ")
                )));
            }
            Err(e) => {
                // R4 Err branch — byte-for-byte preservation of
                // shipped format string at slash.rs:382.
                let _ = ctx
                    .tui_tx
                    .try_send(TuiEvent::Error(format!("Config reload failed: {e}")));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1280.
        "Reload configuration from disk"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B20-RELOAD: tests for /reload slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::{CommandContext, RegistryBuilder};

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `config_path`. Mirrors the AGS-815 `make_ctx(session_id)`
    /// / B17 `make_rename_ctx(session_id)` / B19 `make_rules_ctx(memory)`
    /// shape — DIRECT pattern, no snapshot, no effect slot.
    fn make_reload_ctx(config_path: Option<PathBuf>) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new()
            .with_config_path_opt(config_path)
            .build()
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1280. Any drift here means the two-arg
    /// declare_handler! stub and the new handler have diverged —
    /// Sherlock will flag it.
    #[test]
    fn reload_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            ReloadHandler::new().description(),
            "Reload configuration from disk"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice); spec lists none.
    #[test]
    fn reload_handler_aliases_are_empty() {
        assert!(ReloadHandler::new().aliases().is_empty());
    }

    /// R6: when `config_path` is None, execute returns Err whose
    /// message mentions both `config_path` and `build_command_context`
    /// so the operator can trace the wiring bug. Mirrors the AGS-815
    /// `fork_handler_execute_without_session_id_returns_err` / B17
    /// `execute_without_session_id_returns_err` precedent.
    #[test]
    fn execute_without_config_path_returns_err() {
        let (mut ctx, _rx) = make_reload_ctx(None);
        let h = ReloadHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "ReloadHandler::execute with None config_path must return \
             Err (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("config_path"),
            "Err message must mention 'config_path' so the operator can \
             trace the wiring bug, got: {msg}"
        );
        assert!(
            msg.contains("build_command_context"),
            "Err message must mention 'build_command_context' to pin \
             the owning builder, got: {msg}"
        );
    }

    /// Err-branch integration test. `config_path` points to a path
    /// that does not exist on disk; `force_reload` returns
    /// `ConfigError::ValidationError("no config file found for
    /// reload")` (config_watcher.rs:167). Handler must emit exactly
    /// one `TuiEvent::Error` with the shipped format string
    /// "Config reload failed: {e}" — byte-identical to
    /// slash.rs:382.
    #[tokio::test]
    async fn execute_with_missing_file_emits_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist.toml");
        assert!(!missing.exists());

        let (mut ctx, mut rx) = make_reload_ctx(Some(missing));
        let h = ReloadHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "execute must return Ok(()) even on force_reload failure \
             (errors are surfaced via TuiEvent::Error, not the Result), \
             got: {res:?}"
        );

        let ev = rx.recv().await.expect("Error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.starts_with("Config reload failed: "),
                    "Error event must start with shipped 'Config reload \
                     failed: ' prefix, got: {msg}"
                );
            }
            other => panic!(
                "expected TuiEvent::Error(\"Config reload failed: ...\"), \
                 got: {other:?}"
            ),
        }
    }

    /// Success-path integration test. Write an EMPTY TOML file to a
    /// tempdir and execute the handler. `ArchonConfig` is decorated
    /// with `#[serde(default)]` at every level (config.rs top-level
    /// struct + every nested config type), so an empty TOML file
    /// deserializes to exactly `ArchonConfig::default()`. Because the
    /// handler passes `&ArchonConfig::default()` as the `current`
    /// argument to `force_reload`, `diff_configs` returns an empty
    /// `changed` vec and the handler emits the no-change TextDelta
    /// byte-for-byte. Using an empty file avoids pulling the `toml`
    /// crate into this bin's dev-dependencies (it is a workspace-only
    /// dep) — preserves the byte-identity assertion without a new
    /// Cargo.toml edit.
    #[tokio::test]
    async fn execute_with_valid_config_emits_textdelta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg_path = tmp.path().join("archon.toml");
        // Empty TOML → ArchonConfig::default() via `#[serde(default)]`.
        std::fs::write(&cfg_path, "").expect("write empty toml file");

        let (mut ctx, mut rx) = make_reload_ctx(Some(cfg_path));
        let h = ReloadHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nConfig reloaded. No changes detected.\n",
                    "success-path no-change TextDelta must be byte-identical \
                     to shipped slash.rs:368 format"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nConfig reloaded. No \
                 changes detected.\\n\"), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (success path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/reload` with
    /// `ReloadHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/reload")` routes through the
    /// real alias+primary pipeline. Asserts byte-exact no-change
    /// TextDelta (same empty-file trick as
    /// `execute_with_valid_config_emits_textdelta`).
    #[tokio::test]
    async fn dispatcher_routes_slash_reload_with_config_emits_textdelta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg_path = tmp.path().join("archon.toml");
        std::fs::write(&cfg_path, "").expect("write empty toml file");

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("reload", Arc::new(ReloadHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_reload_ctx(Some(cfg_path));
        let res = dispatcher.dispatch(&mut ctx, "/reload");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for the success \
             path, got: {res:?}"
        );

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nConfig reloaded. No changes detected.\n",
                    "dispatcher-routed no-change TextDelta must be \
                     byte-identical to shipped slash.rs:368 format"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nConfig reloaded. No \
                 changes detected.\\n\"), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/reload` with
    /// `ReloadHandler::new()`, dispatches `"/reload"` with
    /// `config_path: None`, and asserts that `Dispatcher::dispatch`
    /// surfaces the handler's Err (dispatcher.rs:110 forwards
    /// `handler.execute(..)` verbatim — it does NOT swallow
    /// handler-origin Errs).
    #[test]
    fn dispatcher_routes_slash_reload_without_config_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("reload", Arc::new(ReloadHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_reload_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/reload");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must surface handler Err when \
             config_path is None (dispatcher.rs:110 forwards the Err \
             verbatim), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("config_path") && msg.contains("build_command_context"),
            "Err message must mention both 'config_path' and \
             'build_command_context', got: {msg}"
        );
    }
}
