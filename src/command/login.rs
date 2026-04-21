//! TASK-AGS-POST-6-BODIES-B22-LOGIN: /login slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1295` and the legacy match arm at
//! `src/command/slash.rs:285-309`.
//!
//! # R1 — pattern = DIRECT (NOT EFFECT-SLOT as the B22 task tag suggests)
//!
//! Recon of slash.rs:285-309 proved DIRECT is the correct pattern. The
//! shipped `/login` body performs only sync filesystem+string-format
//! work — `dirs::home_dir()`, `.join(...)`, `cred_path.exists()`,
//! `ctx.auth_label` read, and `push_str` building a combined message —
//! plus a single `tui_tx.send(TuiEvent::TextDelta(msg)).await`
//! emission. There is NO OAuth flow, NO credential read, NO async mutex
//! guard, and NO write-back to `SlashCommandContext` state.
//! Consequently:
//!
//! - NO `LoginSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits a single `TuiEvent` — matches AGS-815 /fork and B20
//!   /reload DIRECT precedent).
//! - A NEW `CommandContext::auth_label: Option<String>` field is added
//!   and populated UNCONDITIONALLY by `build_command_context` (mirrors
//!   the AGS-815 `session_id` / AGS-817 `memory` / B20 `config_path`
//!   cross-cutting precedent — not the per-primary SNAPSHOT gating
//!   pattern). `String` clone is cheap; every handler observes this
//!   field for free without a per-command builder match arm.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/login` match arm at slash.rs:285-309 was *async* only
//! because it lived inside the async dispatch loop and emitted via
//! `tui_tx.send(..).await`. The underlying work is 100% sync (no
//! `async fn`, no `.await` in its body — only filesystem existence
//! check and string formatting). In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork + B17 /rename + B20 /reload
//! precedent exactly.
//!
//! # R3 — args ignored (shipped silent-ignore behaviour preserved)
//!
//! The shipped match arm took no args — it matched on the literal
//! `"/login"` string, so trailing tokens like `/login foo` never
//! reached this branch. Under the new registry dispatcher the parser
//! tokenizes `/login foo` into `name = "login"` + `args = ["foo"]` and
//! routes to `LoginHandler::execute`. To preserve the shipped
//! silent-ignore behaviour the handler simply ignores `args` — does
//! NOT emit an error for unexpected arguments. Byte-equivalent to
//! shipped for every possible input that used to reach the arm
//! (`/login` alone), and strictly wider / permissive for inputs that
//! didn't. Matches B20 /reload R3 exactly.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Authenticate against the configured backend"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1295.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and spec lists none.
//! - Emitted events preserve the shipped slash.rs:285-309 format
//!   strings byte-for-byte. Single `TuiEvent::TextDelta(msg)` where
//!   `msg` is built by the same sequence of `push_str` calls across
//!   both branches (authenticated vs not-authenticated).
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B22: none (2-arg `declare_handler!` form at
//! registry.rs:1295). Spec lists none. No aliases added. Matches
//! /fork / /rename / /mcp / /context / /hooks / /reload precedent.
//!
//! # R6 — auth_label unconditional-populate (AGS-815-style)
//!
//! Unlike the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which
//! gate their populate step on the primary name, this ticket extends
//! `CommandContext` with `auth_label: Option<String>` populated
//! UNCONDITIONALLY in `build_command_context` — mirroring the AGS-815
//! `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
//! B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
//! `skill_registry`, B13-GARDEN `garden_config`, and B20-RELOAD
//! `config_path` DIRECT cross-cutting precedent. `String` clone per
//! dispatch is cheap (one heap alloc); future DIRECT handlers that
//! need the auth label inherit this field for free without a
//! per-command builder match arm.
//!
//! The production builder always populates
//! `Some(slash_ctx.auth_label.clone())`. `None` is the sentinel
//! reserved for test fixtures that construct `CommandContext` directly
//! without standing up a full `SlashCommandContext`; in those tests
//! the handler observes `None` and returns an Err-with-message
//! describing the missing-auth_label condition rather than panicking.
//! Mirrors the AGS-815 `fork_handler_execute_without_session_id_returns_err`
//! and B20 `execute_without_config_path_returns_err` pattern.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `LoginHandler` (PATH A,
//! via the dispatcher) AND the legacy `"/login" =>` match arm at
//! slash.rs:285-309 are live. Every `/login` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE subsequent subagent run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.
//!
//! # R8 — existing `handle_login` CLI entry point preserved
//!
//! The pre-existing `handle_login` async fn (extracted from src/main.rs
//! as part of TUI-325) is the CLI-subcommand body for `archon login`
//! (OAuth browser flow). It is called from `main.rs:151` and is
//! entirely unrelated to the slash-command handler. This module keeps
//! both symbols side-by-side; no rename / remove / duplicate-code
//! concern.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::Result;

// ---------------------------------------------------------------------------
// Pre-existing CLI `archon login` entry point (TUI-325). Untouched by B22.
// ---------------------------------------------------------------------------

pub async fn handle_login(_config: &archon_core::config::ArchonConfig) -> Result<()> {
    let http_client = reqwest::Client::new();
    let cred_path = archon_llm::tokens::credentials_path();

    eprintln!("Starting OAuth login...");
    match archon_llm::oauth::login(&cred_path, &http_client).await {
        Ok(_) => {
            eprintln!("Login successful! Credentials saved.");
            Ok(())
        }
        Err(e) => {
            eprintln!("Login failed: {e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B22-LOGIN: slash-command handler.
// ---------------------------------------------------------------------------

/// Zero-sized handler registered as the primary `/login` command.
///
/// No aliases. Shipped pre-B22 stub carried none (2-arg
/// `declare_handler!` form at registry.rs:1295); spec lists none.
/// Matches /fork / /rename / /mcp / /context / /hooks / /reload
/// precedent.
pub(crate) struct LoginHandler;

impl LoginHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`,
    /// `RecallHandler::new`, `RulesHandler::new`, `ReloadHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for LoginHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for LoginHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // R6: require auth_label. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::auth_label`
        // per the AGS-815 session_id / AGS-817 memory / B20 config_path
        // cross-cutting precedent, so at the real dispatch site this
        // branch never fires. Test fixtures that construct
        // `CommandContext` directly with `auth_label: None` will hit
        // this branch and observe an Err — mirroring the AGS-815
        // `fork_handler_execute_without_session_id_returns_err` and
        // B20 `execute_without_config_path_returns_err` pattern.
        let auth_label = ctx.auth_label.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "LoginHandler invoked without ctx.auth_label populated — \
                 build_command_context bug"
            )
        })?;

        // R4: byte-for-byte preservation of slash.rs:285-309 `cred_path`
        // construction.
        let cred_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".archon")
            .join(".credentials.json");

        // R4: byte-for-byte preservation of the slash.rs:291-306 message
        // build. Single `msg` buffer, two branches (authenticated /
        // not-authenticated), single TextDelta emission.
        let mut msg = String::from("\nAuthentication status:\n");
        msg.push_str(&format!("  Method: {}\n", auth_label));
        if cred_path.exists() {
            msg.push_str(&format!("  Credentials: {}\n", cred_path.display()));
            msg.push_str("  Status: authenticated\n\n");
            msg.push_str("  To re-authenticate, run in another terminal:\n");
            msg.push_str("    archon login\n");
        } else {
            msg.push_str("  Credentials: not found\n");
            msg.push_str("  Status: using API key or not authenticated\n\n");
            msg.push_str("  To authenticate with OAuth:\n");
            msg.push_str("    1. Exit this session (Ctrl+D)\n");
            msg.push_str("    2. Run: archon login\n");
            msg.push_str("    3. Follow the browser flow\n");
            msg.push_str("    4. Restart archon\n");
        }

        // R2: sync emission via `try_send`. The shipped arm used
        // `tui_tx.send(..).await` which is forbidden in sync trait
        // methods; best-effort `try_send` matches B17 /rename + B20
        // /reload precedent (dropping a UI message under channel
        // backpressure is preferable to stalling the dispatcher).
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1295.
        "Authenticate against the configured backend"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B22-LOGIN: tests for /login slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::{CommandContext, RegistryBuilder};

    /// Process-wide lock so tests that mutate `HOME` do not race. The
    /// env_guard crate does not serialise arbitrary env vars across
    /// threads, and setting `HOME` is inherently process-global.
    /// `Mutex<()>` suffices — poisoning is tolerated (tests inside
    /// `.lock()` may panic; the next test recovers). Mirrors the
    /// AGS-815 / B20 env-mutation serialisation pattern.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that overrides an env var for the lifetime of a
    /// single test body and restores the prior value on drop.
    /// Not re-entrant. Acquire `ENV_LOCK` FIRST.
    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: Tests serialize via ENV_LOCK so only one EnvGuard
            // is alive at a time per key. The guard restores on Drop.
            // `set_var` is unsafe in edition 2024 because concurrent env
            // mutation is UB on some platforms; ENV_LOCK enforces
            // single-writer discipline for the duration of the test.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: same discipline as `EnvGuard::set` — ENV_LOCK is
            // still held by the test body that owns this guard.
            match self.prev.take() {
                Some(v) => unsafe {
                    std::env::set_var(self.key, v);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `auth_label`. Mirrors the AGS-815 `make_ctx(session_id)`
    /// / B17 `make_rename_ctx(session_id)` / B19 `make_rules_ctx(memory)`
    /// / B20 `make_reload_ctx(config_path)` shape — DIRECT pattern, no
    /// snapshot, no effect slot.
    fn make_login_ctx(
        auth_label: Option<String>,
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
                auth_label,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1295. Any drift here means the two-arg
    /// declare_handler! stub and the new handler have diverged —
    /// Sherlock will flag it.
    #[test]
    fn login_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            LoginHandler::new().description(),
            "Authenticate against the configured backend"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice); spec lists none.
    #[test]
    fn login_handler_aliases_are_empty() {
        assert!(LoginHandler::new().aliases().is_empty());
    }

    /// R6: when `auth_label` is None, execute returns Err whose
    /// message mentions both `auth_label` and `build_command_context`
    /// so the operator can trace the wiring bug. Mirrors the AGS-815
    /// `fork_handler_execute_without_session_id_returns_err` / B17
    /// `execute_without_session_id_returns_err` / B20
    /// `execute_without_config_path_returns_err` precedent.
    #[test]
    fn execute_without_auth_label_returns_err() {
        let (mut ctx, _rx) = make_login_ctx(None);
        let h = LoginHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "LoginHandler::execute with None auth_label must return \
             Err (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("auth_label"),
            "Err message must mention 'auth_label' so the operator can \
             trace the wiring bug, got: {msg}"
        );
        assert!(
            msg.contains("build_command_context"),
            "Err message must mention 'build_command_context' to pin \
             the owning builder, got: {msg}"
        );
    }

    /// Authenticated branch integration test. Override `HOME` to a
    /// tempdir, create `.archon/.credentials.json` there, and assert
    /// the handler emits a single byte-exact TextDelta containing the
    /// `Status: authenticated` block.
    #[tokio::test]
    async fn execute_authenticated_branch_emits_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        let cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");

        let _guard = EnvGuard::set("HOME", tmp.path());

        let (mut ctx, mut rx) =
            make_login_ctx(Some("anthropic-api-key".to_string()));
        let h = LoginHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                let expected = format!(
                    "\nAuthentication status:\n  \
                     Method: anthropic-api-key\n  \
                     Credentials: {}\n  \
                     Status: authenticated\n\n  \
                     To re-authenticate, run in another terminal:\n    \
                     archon login\n",
                    cred_path.display()
                );
                assert_eq!(
                    text, expected,
                    "authenticated-branch TextDelta must be byte-identical \
                     to shipped slash.rs:291-297 format"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(..), got: {other:?}"
            ),
        }
    }

    /// Not-authenticated branch integration test. Override `HOME` to a
    /// tempdir WITHOUT `.archon/.credentials.json`; assert the handler
    /// emits a single byte-exact TextDelta containing the 4-step
    /// OAuth-instructions block.
    #[tokio::test]
    async fn execute_not_authenticated_branch_emits_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        // Do NOT create .archon/.credentials.json — cred_path.exists()
        // must return false.
        let _guard = EnvGuard::set("HOME", tmp.path());

        // Sanity: confirm HOME override propagates to dirs::home_dir().
        let observed = dirs::home_dir()
            .expect("dirs::home_dir returns Some under HOME=tmp");
        assert_eq!(
            observed,
            tmp.path(),
            "dirs::home_dir must reflect the HOME override"
        );
        let cred_path = observed.join(".archon").join(".credentials.json");
        assert!(
            !cred_path.exists(),
            ".archon/.credentials.json must not exist for the not-auth \
             branch, got: {}",
            cred_path.display()
        );

        let (mut ctx, mut rx) =
            make_login_ctx(Some("api-key".to_string()));
        let h = LoginHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                let expected = "\nAuthentication status:\n  \
                     Method: api-key\n  \
                     Credentials: not found\n  \
                     Status: using API key or not authenticated\n\n  \
                     To authenticate with OAuth:\n    \
                     1. Exit this session (Ctrl+D)\n    \
                     2. Run: archon login\n    \
                     3. Follow the browser flow\n    \
                     4. Restart archon\n";
                assert_eq!(
                    text, expected,
                    "not-authenticated-branch TextDelta must be \
                     byte-identical to shipped slash.rs:299-305 format"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(..), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (authenticated path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/login` with
    /// `LoginHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/login")` routes through the
    /// real alias+primary pipeline. Uses `HOME`-override + cred-file
    /// trick to select the authenticated branch deterministically.
    #[tokio::test]
    async fn dispatcher_routes_slash_login_with_auth_label_emits_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        let cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");
        let _guard = EnvGuard::set("HOME", tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("login", Arc::new(LoginHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) =
            make_login_ctx(Some("oauth".to_string()));
        let res = dispatcher.dispatch(&mut ctx, "/login");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert!(
                    text.contains("Method: oauth"),
                    "dispatcher-routed TextDelta must carry the auth_label \
                     through build_command_context → execute, got: {text}"
                );
                assert!(
                    text.contains("Status: authenticated"),
                    "dispatcher-routed TextDelta must reflect the \
                     authenticated branch (cred_path exists), got: {text}"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(..), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/login` with
    /// `LoginHandler::new()`, dispatches `"/login"` with
    /// `auth_label: None`, and asserts that `Dispatcher::dispatch`
    /// surfaces the handler's Err (dispatcher forwards
    /// `handler.execute(..)` verbatim — it does NOT swallow
    /// handler-origin Errs).
    #[test]
    fn dispatcher_routes_slash_login_without_auth_label_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary("login", Arc::new(LoginHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_login_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/login");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must surface handler Err when \
             auth_label is None, got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("auth_label")
                && msg.contains("build_command_context"),
            "Err message must mention both 'auth_label' and \
             'build_command_context', got: {msg}"
        );
    }
}
