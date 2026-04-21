//! TASK-AGS-POST-6-BODIES-B17-RENAME: /rename slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1234` and the legacy match arm at
//! `src/command/slash.rs:422-462`.
//!
//! # R1 — pattern = DIRECT (no snapshot, no effect slot)
//!
//! The shipped `/rename` body reads `archon_session` synchronously —
//! both `archon_session::storage::SessionStore::open` and
//! `archon_session::naming::set_session_name` are plain sync
//! functions. No `tokio::sync::Mutex` guards on the read path and
//! no writes back to `SlashCommandContext` state. Consequently:
//!
//! - NO `RenameSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits `TuiEvent`s — matches AGS-815 /fork precedent).
//! - NO `build_command_context` match arm added for `/rename`. Unlike
//!   the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which gate
//!   their populate step on the primary name, AGS-815 /fork already
//!   extended `CommandContext` with `session_id: Option<String>`
//!   populated UNCONDITIONALLY in the builder. `/rename` reuses that
//!   exact field — no new context.rs wiring required for this ticket.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/rename` match arm at slash.rs:422-462 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying archon-session calls
//! are 100% sync. In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used `s.strip_prefix("/rename").unwrap_or("").trim()`
//! on the full input string, so `/rename my new name` (three tokens)
//! was forwarded verbatim as the session name `"my new name"`. The
//! registry parser tokenizes on whitespace, so `args` is `["my",
//! "new", "name"]` — three entries. To preserve the shipped
//! single-string semantics while going through the parser, the handler
//! joins `args` with a single space then `.trim()`s. This is
//! byte-equivalent to the shipped behaviour for all inputs:
//! single-token args pass through unchanged, multi-token args preserve
//! whitespace-joined substring, empty args → empty string → usage
//! error. See `src/command/add_dir.rs:155-180` for the same pattern.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Rename the current session"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1234.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and the spec lists
//!   none.
//! - Emitted events preserve the shipped slash.rs:422-462 format
//!   strings byte-for-byte:
//!   * Empty-arg → `TuiEvent::Error("Usage: /rename <name>")`.
//!   * Rename success → `TuiEvent::SessionRenamed(name_arg.to_string())`
//!     followed by `TuiEvent::TextDelta(format!("\nSession renamed to:
//!     {name_arg}\n"))` — in that order.
//!   * `set_session_name` error → `TuiEvent::Error(format!("Rename
//!     failed: {e}"))`.
//!   * `SessionStore::open` error → `TuiEvent::Error(format!("Session
//!     store error: {e}"))`.
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B17: none (2-arg declare_handler! form at
//! registry.rs:1234). Spec lists none. No aliases added. Matches
//! /fork / /mcp / /context / /hooks precedent.
//!
//! # R6 — session_id reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::session_id: Option<String>` is already populated
//! unconditionally by `build_command_context` per AGS-815 /fork.rs.
//! This ticket REUSES that exact field — there is no
//! `rename_snapshot` type, no context.rs match arm added, no new
//! `build_command_context` wiring. The test fixture helper
//! (`make_rename_ctx`) mirrors the AGS-815 /fork `make_ctx(session_id)`
//! shape.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `RenameHandler` (PATH A,
//! via the dispatcher at slash.rs:46) AND the legacy `s if
//! s.starts_with("/rename")` match arm at slash.rs:422-462 are live.
//! Every `/rename` invocation therefore fires twice — once via the
//! handler and once via the legacy arm. This is the Stage-6
//! body-migrate protocol: Gate 5 deletes the legacy match arm in a
//! SEPARATE subsequent subagent run (NOT this subagent's
//! responsibility). Do NOT touch slash.rs in this ticket.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/rename` command.
///
/// No aliases. Shipped pre-B17 stub carried none (2-arg
/// declare_handler! form); spec lists none. Matches /fork / /mcp /
/// /context / /hooks precedent.
pub(crate) struct RenameHandler;

impl RenameHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`) even though the
    /// unit struct is constructible without it — the explicit
    /// constructor keeps the call site in registry.rs:1327
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for RenameHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for RenameHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R3: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/rename").unwrap_or("").trim()`
        // for all inputs — single-token names collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // names preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the usage-error branch identical to the shipped
        // `if name_arg.is_empty()` check at slash.rs:424.
        let joined = args.join(" ");
        let name_arg = joined.trim();

        if name_arg.is_empty() {
            // Empty-arg branch — byte-for-byte preservation of shipped
            // format string at slash.rs:426.
            let _ = ctx
                .tui_tx
                .try_send(TuiEvent::Error("Usage: /rename <name>".into()));
            return Ok(());
        }

        // R6: require session_id. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::session_id`
        // per the AGS-815 fork.rs precedent, so at the real dispatch
        // site this branch never fires. Test fixtures that construct
        // `CommandContext` directly with `session_id: None` will hit
        // this branch and observe an Err — mirroring the
        // `fork_handler_execute_without_session_id_returns_err` pattern
        // established in AGS-815.
        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "RenameHandler invoked without ctx.session_id populated — \
                 build_command_context bug"
            )
        })?;

        // Open the session store. Every downstream branch depends on a
        // valid `SessionStore`; a failure here surfaces as a
        // user-facing `TuiEvent::Error`, matching the shipped Err arm
        // at slash.rs:454-458.
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                match archon_session::naming::set_session_name(
                    &store, session_id, name_arg,
                ) {
                    Ok(()) => {
                        // Success path: emit SessionRenamed first
                        // (consumers gate on this variant for TUI
                        // state), then the human-readable TextDelta.
                        // Byte-identical to shipped slash.rs:437-445
                        // order.
                        let _ = ctx.tui_tx.try_send(
                            TuiEvent::SessionRenamed(name_arg.to_string()),
                        );
                        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                            format!("\nSession renamed to: {name_arg}\n"),
                        ));
                    }
                    Err(e) => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                            format!("Rename failed: {e}"),
                        ));
                    }
                }
            }
            Err(e) => {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                    "Session store error: {e}"
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1234.
        "Rename the current session"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B17-RENAME: tests for /rename slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, OnceLock};

    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::{CommandContext, RegistryBuilder};

    /// Serialize env-mutating tests across `--test-threads=N` (N>1).
    /// `default_db_path()` reads `dirs::data_dir()` which honours
    /// `XDG_DATA_HOME` then falls back to `HOME/.local/share` on Linux.
    /// Any test that needs the handler's `archon_session::storage::
    /// default_db_path()` call to resolve to a tempdir MUST take this
    /// lock for the duration of the env mutation + handler call.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// RAII guard that sets `XDG_DATA_HOME` + `HOME` to the supplied
    /// tempdir on construction and restores the prior values on drop.
    /// The caller must hold `env_lock()` for the guard's lifetime to
    /// prevent cross-test env races under `--test-threads>1`.
    struct EnvGuard {
        prev_xdg: Option<std::ffi::OsString>,
        prev_home: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(tmp: &std::path::Path) -> Self {
            let g = Self {
                prev_xdg: std::env::var_os("XDG_DATA_HOME"),
                prev_home: std::env::var_os("HOME"),
            };
            // SAFETY: env mutation is protected by the
            // process-global `env_lock()` Mutex acquired by every
            // caller. No other thread mutates XDG_DATA_HOME/HOME
            // while this guard is alive.
            unsafe {
                std::env::set_var("XDG_DATA_HOME", tmp);
                std::env::set_var("HOME", tmp);
            }
            g
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see `EnvGuard::set`. Lock still held by caller.
            unsafe {
                match self.prev_xdg.take() {
                    Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                    None => std::env::remove_var("XDG_DATA_HOME"),
                }
                match self.prev_home.take() {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `session_id`. Mirrors the `make_ctx(session_id)`
    /// fixture in `src/command/fork.rs` — DIRECT pattern, no
    /// snapshot, no effect slot.
    fn make_rename_ctx(
        session_id: Option<String>,
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
                session_id,
                memory: None,
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
    /// stub at registry.rs:1234. Any drift here means the two-arg
    /// declare_handler! stub and the new handler have diverged —
    /// Sherlock will flag it.
    #[test]
    fn rename_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            RenameHandler::new().description(),
            "Rename the current session"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice); spec lists none.
    #[test]
    fn rename_handler_aliases_are_empty() {
        assert_eq!(RenameHandler::new().aliases(), &[] as &[&str]);
    }

    /// Empty args: emit the usage-error TuiEvent (byte-identical to
    /// shipped slash.rs:426) and return Ok(()). No session_id lookup
    /// is performed (the empty-args branch short-circuits BEFORE the
    /// session_id check, matching shipped control flow).
    #[tokio::test]
    async fn execute_with_empty_args_emits_usage_error() {
        let (mut ctx, mut rx) = make_rename_ctx(None);
        let h = RenameHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "empty-args branch must return Ok(()) (event emission is \
             best-effort via try_send), got: {res:?}"
        );
        let ev = rx.recv().await.expect("usage error must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(msg, "Usage: /rename <name>");
            }
            other => panic!(
                "expected TuiEvent::Error(\"Usage: /rename <name>\"), \
                 got: {other:?}"
            ),
        }
    }

    /// R6: when `session_id` is None but args are non-empty, execute
    /// returns Err whose message mentions both `session_id` and
    /// `build_command_context` so the operator can trace the wiring
    /// bug. Mirrors the AGS-815 fork_handler_execute_without_session_id
    /// precedent.
    #[test]
    fn execute_without_session_id_returns_err() {
        let (mut ctx, _rx) = make_rename_ctx(None);
        let h = RenameHandler::new();
        let res = h.execute(&mut ctx, &["mynewname".to_string()]);
        assert!(
            res.is_err(),
            "RenameHandler::execute with None session_id and non-empty \
             args must return Err (builder contract violation), \
             got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("session_id"),
            "Err message must mention 'session_id' so the operator can \
             trace the wiring bug, got: {msg}"
        );
        assert!(
            msg.contains("build_command_context"),
            "Err message must mention 'build_command_context' to pin \
             the owning builder, got: {msg}"
        );
    }

    /// Success-path integration test. Redirects
    /// `default_db_path()` to a tempdir-backed sqlite DB via
    /// XDG_DATA_HOME+HOME env mutation under an `env_lock()` guard,
    /// executes the handler with a non-empty name arg, and asserts
    /// BOTH `TuiEvent::SessionRenamed(name)` AND
    /// `TuiEvent::TextDelta("\nSession renamed to: {name}\n")` are
    /// emitted in that order.
    ///
    /// CHOICE: real-store path (per prompt option 1). The
    /// tempdir+SessionStore setup is cheap (< 50ms on modern
    /// hardware) and exercises the full success branch, which no
    /// other test covers. `set_session_name` writes to the
    /// `session_names` relation which has no FK into `sessions`
    /// (storage.rs:704-712 `:put session_names {session_id => name}`),
    /// so NO `create_session` prerequisite is required — the
    /// session_id string is inserted verbatim.
    ///
    /// env_lock() serializes this test with
    /// `dispatcher_routes_slash_rename_with_session_id_emits_expected_events`
    /// under `--test-threads=2` — both mutate process-global
    /// XDG_DATA_HOME/HOME, which must not race.
    #[tokio::test]
    async fn execute_with_session_id_success_path_emits_events() {
        let _env_guard = env_lock().lock().expect("env_lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let sid = "test-session-b17-direct";
        let (mut ctx, mut rx) = make_rename_ctx(Some(sid.to_string()));
        let h = RenameHandler::new();
        let res = h.execute(&mut ctx, &["mynewname".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev1 =
            rx.recv().await.expect("SessionRenamed event must be emitted");
        match ev1 {
            TuiEvent::SessionRenamed(name) => {
                assert_eq!(name, "mynewname");
            }
            other => panic!(
                "expected TuiEvent::SessionRenamed(\"mynewname\") first, \
                 got: {other:?}"
            ),
        }
        let ev2 =
            rx.recv().await.expect("TextDelta confirmation must be emitted");
        match ev2 {
            TuiEvent::TextDelta(text) => {
                assert_eq!(text, "\nSession renamed to: mynewname\n");
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nSession renamed to: \
                 mynewname\\n\") second, got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (success path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/rename` with
    /// `RenameHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/rename dispatchedname")`
    /// routes through the real alias+primary pipeline. Uses the same
    /// XDG_DATA_HOME/HOME override scheme +
    /// `env_lock()` serialization as
    /// `execute_with_session_id_success_path_emits_events`.
    #[tokio::test]
    async fn dispatcher_routes_slash_rename_with_session_id_emits_expected_events(
    ) {
        let _env_guard = env_lock().lock().expect("env_lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("rename", Arc::new(RenameHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let sid = "test-session-b17-dispatch";
        let (mut ctx, mut rx) = make_rename_ctx(Some(sid.to_string()));
        let res = dispatcher.dispatch(&mut ctx, "/rename dispatchedname");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for the success \
             path, got: {res:?}"
        );

        let ev1 =
            rx.recv().await.expect("SessionRenamed must be emitted");
        match ev1 {
            TuiEvent::SessionRenamed(name) => {
                assert_eq!(name, "dispatchedname");
            }
            other => panic!(
                "expected TuiEvent::SessionRenamed(\"dispatchedname\"), \
                 got: {other:?}"
            ),
        }
        let ev2 = rx.recv().await.expect("TextDelta must be emitted");
        match ev2 {
            TuiEvent::TextDelta(text) => {
                assert_eq!(text, "\nSession renamed to: dispatchedname\n");
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nSession renamed to: \
                 dispatchedname\\n\"), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/rename` with
    /// `RenameHandler::new()`, dispatches `"/rename somename"` with
    /// `session_id: None`, and asserts that `Dispatcher::dispatch`
    /// surfaces the handler's Err (dispatcher.rs:110 forwards
    /// `handler.execute(..)` verbatim — it does NOT swallow
    /// handler-origin Errs).
    #[test]
    fn dispatcher_routes_slash_rename_without_session_id_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("rename", Arc::new(RenameHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_rename_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/rename somename");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must surface handler Err when \
             session_id is None (dispatcher.rs:110 forwards the Err \
             verbatim), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("session_id")
                && msg.contains("build_command_context"),
            "Err message must mention both 'session_id' and \
             'build_command_context', got: {msg}"
        );
    }
}
