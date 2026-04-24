//! TASK-AGS-POST-6-BODIES-B23-LOGOUT: /logout slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1393` and the legacy match arm at
//! `src/command/slash.rs:365-392`.
//!
//! # R1 — pattern = DIRECT (NOT EFFECT-SLOT as the B23 task tag suggests)
//!
//! Recon of slash.rs:365-392 proved DIRECT is the correct pattern. The
//! shipped `/logout` body performs only sync filesystem work —
//! `dirs::home_dir()`, `.join(...)`, `cred_path.exists()`, and
//! `std::fs::remove_file` — plus three `tui_tx.send(..).await`
//! emissions across its three branches. There is NO OAuth flow, NO
//! async I/O, NO async mutex guard, and NO write-back to
//! `SlashCommandContext` state. Consequently:
//!
//! - NO `LogoutSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it emits `TuiEvent` values only — matches AGS-815 /fork, B20
//!   /reload, and B22 /login DIRECT precedent).
//! - NO new `CommandContext` field (unlike B22 /login which added
//!   `auth_label`: /logout reads no cross-cutting state. The only
//!   runtime signal is the presence or absence of
//!   `~/.archon/.credentials.json` on disk, which is resolved inline
//!   via `dirs::home_dir()` — no builder involvement required).
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/logout` match arm at slash.rs:365-392 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying work is 100% sync (no
//! `async fn`, no `.await` in its body — only `dirs::home_dir()`,
//! `Path::exists()`, `std::fs::remove_file()` — all synchronous). In
//! the new sync handler we emit via `ctx.tui_tx.try_send(..)` (best-
//! effort — dropping a UI message under channel backpressure is
//! preferable to stalling the dispatcher). Matches B17 /rename + B20
//! /reload + B22 /login precedent exactly.
//!
//! # R3 — args ignored (shipped silent-ignore behaviour preserved)
//!
//! The shipped match arm took no args — it matched on the literal
//! `"/logout"` string, so trailing tokens like `/logout foo` never
//! reached this branch. Under the new registry dispatcher the parser
//! tokenizes `/logout foo` into `name = "logout"` + `args = ["foo"]`
//! and routes to `LogoutHandler::execute`. To preserve the shipped
//! silent-ignore behaviour the handler simply ignores `args` — does
//! NOT emit an error for unexpected arguments. Byte-equivalent to
//! shipped for every possible input that used to reach the arm
//! (`/logout` alone), and strictly wider / permissive for inputs that
//! didn't. Matches B20 /reload and B22 /login R3 exactly.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Clear stored credentials"` — byte-
//!   identical to the `declare_handler!` stub at registry.rs:1393.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and spec lists none.
//! - Emitted events preserve the shipped slash.rs:365-392 format
//!   strings byte-for-byte across all three branches:
//!     1. cred_path exists + remove Ok -> TextDelta("\nLogged out.
//!        Credentials cleared.\nRestart and run /login to
//!        re-authenticate.\n")
//!     2. cred_path exists + remove Err -> Error(format!(
//!        "Failed to clear credentials: {e}"))
//!     3. !cred_path.exists() -> TextDelta("\nNo stored credentials
//!        found. Using API key auth.\n")
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B23: none (2-arg `declare_handler!` form at
//! registry.rs:1393). Spec lists none. No aliases added. Matches
//! /fork / /rename / /mcp / /context / /hooks / /reload / /login
//! precedent.
//!
//! # R6 — no CommandContext field added
//!
//! Unlike B22 /login which added `auth_label: Option<String>` as a
//! cross-cutting DIRECT field, /logout reads no shared state. The
//! filesystem probe resolves against `dirs::home_dir()` inline — no
//! builder involvement, no new field, no fixture churn. The handler
//! is effectively stateless with respect to `CommandContext` beyond
//! the `tui_tx` channel every handler uses to emit events.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `LogoutHandler` (PATH A,
//! via the dispatcher) AND the legacy `"/logout" =>` match arm at
//! slash.rs:365-392 are live. Every `/logout` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE parent-context run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: slash-command handler.
// ---------------------------------------------------------------------------

/// Zero-sized handler registered as the primary `/logout` command.
///
/// No aliases. Shipped pre-B23 stub carried none (2-arg
/// `declare_handler!` form at registry.rs:1393); spec lists none.
/// Matches /fork / /rename / /mcp / /context / /hooks / /reload /
/// /login precedent.
pub(crate) struct LogoutHandler;

impl LogoutHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`,
    /// `RecallHandler::new`, `RulesHandler::new`, `ReloadHandler::new`,
    /// `LoginHandler::new`) even though the unit struct is
    /// constructible without it — the explicit constructor keeps the
    /// call site in registry.rs copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for LogoutHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for LogoutHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // R4: byte-for-byte preservation of slash.rs:365-370 `cred_path`
        // construction.
        let cred_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".archon")
            .join(".credentials.json");

        // R4: byte-for-byte preservation of the slash.rs:371-390 three-
        // branch emission shape. Sync `Path::exists()` gate; sync
        // `std::fs::remove_file()` on the authenticated branch; single
        // fallback TextDelta on the unauthenticated branch.
        if cred_path.exists() {
            match std::fs::remove_file(&cred_path) {
                Ok(()) => {
                    // R2: sync emission via `try_send`. The shipped arm
                    // used `tui_tx.send(..).await` which is forbidden in
                    // sync trait methods; best-effort `try_send` matches
                    // B17 /rename + B20 /reload + B22 /login precedent.
                    ctx.emit(TuiEvent::TextDelta(
                        "\nLogged out. Credentials cleared.\n\
                         Restart and run /login to re-authenticate.\n"
                            .into(),
                    ));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!("Failed to clear credentials: {e}")));
                }
            }
        } else {
            ctx.emit(TuiEvent::TextDelta(
                "\nNo stored credentials found. Using API key auth.\n".into(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1393.
        "Clear stored credentials"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: tests for /logout slash-command body-migrate
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
    /// AGS-815 / B20 / B22 env-mutation serialisation pattern.
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

    /// Build a `CommandContext` with a freshly-created channel. /logout
    /// adds no new CommandContext field, so every optional field is
    /// `None` — mirroring `make_bug_ctx` (the other new-field-free
    /// handler). No `auth_label` argument needed.
    fn make_logout_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1393. Any drift here means the two-arg
    /// declare_handler! stub and the new handler have diverged —
    /// Sherlock will flag it.
    #[test]
    fn logout_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            LogoutHandler::new().description(),
            "Clear stored credentials"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice); spec lists none.
    #[test]
    fn logout_handler_aliases_are_empty() {
        assert!(LogoutHandler::new().aliases().is_empty());
    }

    /// R4 branch 3: `cred_path.exists() == false` emits the
    /// byte-exact no-stored-credentials TextDelta. HOME points at a
    /// tempdir WITHOUT `.archon/.credentials.json`.
    #[tokio::test]
    async fn execute_no_credentials_emits_no_stored_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        // Do NOT create .archon/.credentials.json — cred_path.exists()
        // must return false.
        let _guard = EnvGuard::set("HOME", tmp.path());

        // Sanity: confirm HOME override propagates to dirs::home_dir().
        let observed = dirs::home_dir().expect("dirs::home_dir returns Some under HOME=tmp");
        assert_eq!(
            observed,
            tmp.path(),
            "dirs::home_dir must reflect the HOME override"
        );
        let cred_path = observed.join(".archon").join(".credentials.json");
        assert!(
            !cred_path.exists(),
            ".archon/.credentials.json must not exist for the no-creds \
             branch, got: {}",
            cred_path.display()
        );

        let (mut ctx, mut rx) = make_logout_ctx();
        let h = LogoutHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nNo stored credentials found. Using API key auth.\n",
                    "no-creds-branch TextDelta must be byte-identical \
                     to shipped slash.rs:385-389 format"
                );
            }
            other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
        }
    }

    /// R4 branch 1: `cred_path.exists() == true` + `remove_file`
    /// succeeds -> emit the byte-exact logged-out TextDelta AND the
    /// file is gone post-execute.
    #[tokio::test]
    async fn execute_remove_success_emits_logged_out_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        let cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");

        let _guard = EnvGuard::set("HOME", tmp.path());

        // Sanity: the file must exist pre-execute.
        assert!(
            cred_path.exists(),
            "cred_path must exist before execute, got: {}",
            cred_path.display()
        );

        let (mut ctx, mut rx) = make_logout_ctx();
        let h = LogoutHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text,
                    "\nLogged out. Credentials cleared.\n\
                     Restart and run /login to re-authenticate.\n",
                    "logged-out-branch TextDelta must be byte-identical \
                     to shipped slash.rs:373-376 format"
                );
            }
            other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
        }

        // Post-execute: the file must be gone.
        assert!(
            !cred_path.exists(),
            "cred_path must be removed after execute, still at: {}",
            cred_path.display()
        );
    }

    /// R4 branch 2: `cred_path.exists() == true` + `remove_file` fails
    /// -> emit the byte-structure-exact Error event carrying the
    /// `format!("Failed to clear credentials: {e}")` payload.
    ///
    /// Forcing `remove_file` to fail deterministically: make the path
    /// a directory rather than a regular file. `std::fs::remove_file`
    /// returns an Err on a directory ("Is a directory" on Linux /
    /// EISDIR).
    #[tokio::test]
    async fn execute_remove_failure_emits_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        // Create `.credentials.json` as a DIRECTORY so `remove_file`
        // fails at runtime. `cred_path.exists()` still returns true
        // (it's a directory entry).
        let cred_path = archon_dir.join(".credentials.json");
        std::fs::create_dir_all(&cred_path).expect("create .credentials.json as dir");

        let _guard = EnvGuard::set("HOME", tmp.path());

        // Sanity: cred_path exists-as-directory.
        assert!(
            cred_path.exists(),
            "cred_path must exist before execute, got: {}",
            cred_path.display()
        );
        assert!(
            cred_path.is_dir(),
            "cred_path must be a directory to force remove_file \
             failure, got: {}",
            cred_path.display()
        );

        let (mut ctx, mut rx) = make_logout_ctx();
        let h = LogoutHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx.recv().await.expect("Error must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.starts_with("Failed to clear credentials: "),
                    "Error payload must be format!(\"Failed to clear \
                     credentials: {{e}}\"); got: {msg}"
                );
                // The suffix is the OS error message — don't pin its
                // exact text across platforms, but assert it is non-
                // empty so we know `{e}` rendered something.
                let suffix = &msg["Failed to clear credentials: ".len()..];
                assert!(
                    !suffix.is_empty(),
                    "Error payload suffix must carry the OS error \
                     rendering of the std::io::Error, got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::Error(..), got: {other:?}"),
        }
    }

    /// Dispatcher-integration test (no-creds branch). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/logout` with
    /// `LogoutHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/logout")` routes through the
    /// real alias+primary pipeline. Uses `HOME`-override (no cred
    /// file) to select the no-creds branch deterministically.
    #[tokio::test]
    async fn dispatcher_routes_slash_logout_no_creds_emits_textdelta() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        // No cred file.
        let _guard = EnvGuard::set("HOME", tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("logout", Arc::new(LogoutHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_logout_ctx();
        let res = dispatcher.dispatch(&mut ctx, "/logout");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "\nNo stored credentials found. Using API key auth.\n",
                    "dispatcher-routed TextDelta must be byte-identical \
                     to shipped slash.rs:385-389 format"
                );
            }
            other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
        }
    }

    /// Dispatcher-integration test (remove-success branch). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/logout` with
    /// `LogoutHandler::new()`, then
    /// `Dispatcher::dispatch(&mut ctx, "/logout")` routes through the
    /// real alias+primary pipeline. Seeded cred file proves the
    /// logged-out branch fires end-to-end AND the file is gone after.
    #[tokio::test]
    async fn dispatcher_routes_slash_logout_removes_creds() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        let cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");
        let _guard = EnvGuard::set("HOME", tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("logout", Arc::new(LogoutHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_logout_ctx();
        let res = dispatcher.dispatch(&mut ctx, "/logout");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );

        let ev = rx.recv().await.expect("TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text,
                    "\nLogged out. Credentials cleared.\n\
                     Restart and run /login to re-authenticate.\n",
                    "dispatcher-routed TextDelta must be byte-identical \
                     to shipped slash.rs:373-376 format"
                );
            }
            other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
        }

        // Post-dispatch: file must be gone.
        assert!(
            !cred_path.exists(),
            "cred_path must be removed after dispatch, still at: {}",
            cred_path.display()
        );
    }
}
