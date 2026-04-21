//! TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: /checkpoint slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs:1361` and the legacy match arm at
//! `src/command/slash.rs:452-527`.
//!
//! # R1 — pattern = DIRECT (not EFFECT-SLOT)
//!
//! Parent-context recon proved every touched archon-session entry point
//! is sync (not async as the B21 task tag suggested):
//!
//! - `archon_session::checkpoint::CheckpointStore::open(&Path)` — sync
//!   (checkpoint.rs:83).
//! - `CheckpointStore::list_modified(&str)` — sync (checkpoint.rs:244).
//! - `CheckpointStore::restore(&str, &str)` — sync (checkpoint.rs:198).
//!
//! Consequently:
//!
//! - NO `CheckpointSnapshot` type (nothing to pre-compute inside an
//!   async guard).
//! - NO `CommandEffect` variant (handler never mutates shared
//!   SlashCommandContext state; it only emits `TuiEvent`s — matches
//!   AGS-815 /fork and B17 /rename precedent).
//! - NO `build_command_context` match arm added. Like /fork (AGS-815)
//!   and /rename (B17), /checkpoint reuses the UNCONDITIONAL
//!   `CommandContext::session_id: Option<String>` field populated by
//!   the builder. No new context.rs wiring required.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/checkpoint` match arm at slash.rs:452-527 was *async*
//! only because it emitted via `tui_tx.send(..).await`. Every
//! archon-session call beneath it is 100% sync. In the new sync handler
//! we emit via `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI
//! message under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork and B17 /rename precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used
//! `s.strip_prefix("/checkpoint").unwrap_or("").trim()`
//! on the full input string, so `/checkpoint restore some/file.txt`
//! (two post-primary tokens) was forwarded verbatim as the arg string
//! `"restore some/file.txt"`. The registry parser tokenizes on
//! whitespace, so `args` is `["restore", "some/file.txt"]`. To preserve
//! shipped single-string semantics we `args.join(" ").trim()`, then
//! match the original branching (`arg == "list" || arg.is_empty()` /
//! `arg.strip_prefix("restore").map(|s| s.trim())`). Byte-equivalent to
//! shipped for all inputs; matches B17 /rename and B18 /recall
//! precedent.
//!
//! # R4 — byte-identity of all 8 event branches
//!
//! Preserved from slash.rs:452-527 byte-for-byte:
//!
//! 1. list / empty + Ok(empty) →
//!    `TuiEvent::TextDelta("\nNo checkpoints for this session.\n")`.
//! 2. list / empty + Ok(non-empty) → `TuiEvent::TextDelta` starting
//!    `"\nCheckpoints:\n"` then per-entry
//!    `format!("  turn {} | {} | {} | {}\n", s.turn_number, s.tool_name,
//!     s.file_path, s.timestamp)`.
//! 3. list / empty + Err → `TuiEvent::Error(format!("Checkpoint list
//!    error: {e}"))`.
//! 4. `restore` + empty path →
//!    `TuiEvent::Error("Usage: /checkpoint restore <file_path>")`.
//! 5. `restore <p>` + Ok → `TuiEvent::TextDelta(format!("\nRestored:
//!    {file_path}\n"))`.
//! 6. `restore <p>` + Err → `TuiEvent::Error(format!("Restore failed:
//!    {e}"))`.
//! 7. Store-open Err (both list and restore paths) →
//!    `TuiEvent::Error(format!("Checkpoint store error: {e}"))`.
//! 8. Catch-all (non-list, non-empty, non-"restore ...") →
//!    `TuiEvent::TextDelta("\nUsage: /checkpoint list | /checkpoint
//!    restore <file_path>\n")` — NOTE: `TextDelta` not `Error`, this is
//!    byte-identical to shipped.
//!
//! `description()` returns `"Create or restore a session checkpoint"` —
//! byte-identical to `declare_handler!` stub at registry.rs:1361.
//! `aliases()` returns `&[]` — shipped stub used the 2-arg form.
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B21: none (2-arg declare_handler! form). Spec lists
//! none. No aliases added. Matches /fork / /rename / /mcp / /context
//! precedent.
//!
//! # R6 — session_id reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::session_id: Option<String>` is already populated
//! unconditionally by `build_command_context` per AGS-815 /fork. This
//! ticket REUSES that exact field — there is no `checkpoint_snapshot`
//! type, no context.rs match arm added, no new `build_command_context`
//! wiring. Test fixtures pass `session_id: Some(..)` directly.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `CheckpointHandler` (PATH
//! A, via the dispatcher at slash.rs:46) AND the legacy `s if s ==
//! "/checkpoint" || s.starts_with("/checkpoint ")` match arm at
//! slash.rs:452-527 are live. Every `/checkpoint` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE subsequent subagent run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.

use std::path::PathBuf;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/checkpoint` command.
///
/// No aliases. Shipped pre-B21 stub carried none (2-arg
/// declare_handler! form); spec lists none. Matches /fork / /rename /
/// /mcp / /context / /hooks precedent.
pub(crate) struct CheckpointHandler;

impl CheckpointHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`RenameHandler::new`, `DoctorHandler::new`, `UsageHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs:1467
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for CheckpointHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for CheckpointHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R6: require session_id. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::session_id`
        // per the AGS-815 fork.rs precedent, so at the real dispatch
        // site this branch never fires. Test fixtures that construct
        // `CommandContext` directly with `session_id: None` will hit
        // this branch and observe an Err — mirroring the
        // `fork_handler_execute_without_session_id_returns_err` and
        // B17 rename pattern.
        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "CheckpointHandler invoked without ctx.session_id populated — \
                 build_command_context bug"
            )
        })?;

        // R3: reconstruct the single arg-string from positional tokens.
        // Byte-equivalent to shipped `s.strip_prefix("/checkpoint")
        // .unwrap_or("").trim()` for all inputs.
        let joined = args.join(" ");
        let arg = joined.trim();

        // R4: path reproduced byte-identically from shipped slash.rs:455-458.
        let ckpt_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("checkpoints.db");

        if arg == "list" || arg.is_empty() {
            match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                Ok(store) => match store.list_modified(session_id) {
                    Ok(snapshots) if snapshots.is_empty() => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                            "\nNo checkpoints for this session.\n".into(),
                        ));
                    }
                    Ok(snapshots) => {
                        let mut out = String::from("\nCheckpoints:\n");
                        for s in &snapshots {
                            out.push_str(&format!(
                                "  turn {} | {} | {} | {}\n",
                                s.turn_number,
                                s.tool_name,
                                s.file_path,
                                s.timestamp
                            ));
                        }
                        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(out));
                    }
                    Err(e) => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                            "Checkpoint list error: {e}"
                        )));
                    }
                },
                Err(e) => {
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                        "Checkpoint store error: {e}"
                    )));
                }
            }
        } else if let Some(file_path) =
            arg.strip_prefix("restore").map(|s| s.trim())
        {
            if file_path.is_empty() {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                    "Usage: /checkpoint restore <file_path>".into(),
                ));
            } else {
                match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                    Ok(store) => match store.restore(session_id, file_path) {
                        Ok(()) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                                format!("\nRestored: {file_path}\n"),
                            ));
                        }
                        Err(e) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                                format!("Restore failed: {e}"),
                            ));
                        }
                    },
                    Err(e) => {
                        let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                            "Checkpoint store error: {e}"
                        )));
                    }
                }
            }
        } else {
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                "\nUsage: /checkpoint list | /checkpoint restore <file_path>\n"
                    .into(),
            ));
        }

        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at registry.rs:1361.
        "Create or restore a session checkpoint"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: tests for /checkpoint body-migrate
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
    /// `dirs::data_dir()` honours `XDG_DATA_HOME` then falls back to
    /// `HOME/.local/share` on Linux. Any test that needs the handler's
    /// `dirs::data_dir().join("archon").join("checkpoints.db")` call
    /// to resolve under a tempdir MUST take this lock for the duration
    /// of the env mutation + handler call. Mirrors B17 /rename
    /// `env_lock()` precedent at src/command/rename.rs:239.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// RAII guard that sets `XDG_DATA_HOME` + `HOME` to the supplied
    /// tempdir on construction and restores the prior values on drop.
    /// The caller must hold `env_lock()` for the guard's lifetime to
    /// prevent cross-test env races under `--test-threads>1`. Mirrors
    /// B17 /rename `EnvGuard` precedent at
    /// src/command/rename.rs:248.
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
            // SAFETY: env mutation is protected by the process-global
            // `env_lock()` Mutex acquired by every caller. No other
            // thread mutates XDG_DATA_HOME/HOME while this guard is alive.
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
    /// supplied `session_id`. Mirrors the `make_rename_ctx(session_id)`
    /// fixture in `src/command/rename.rs` — DIRECT pattern, no
    /// snapshot, no effect slot.
    fn make_ckpt_ctx(
        session_id: Option<String>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new()
            .with_session_id_opt(session_id)
            .build()
    }

    /// R4: description is byte-identical to the `declare_handler!`
    /// stub at registry.rs:1361. Any drift here means the two-arg
    /// declare_handler! stub and the new handler have diverged —
    /// Sherlock will flag it.
    #[test]
    fn checkpoint_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            CheckpointHandler::new().description(),
            "Create or restore a session checkpoint"
        );
    }

    /// R5: zero aliases. Shipped stub used the 2-arg
    /// `declare_handler!` form (no aliases slice); spec lists none.
    #[test]
    fn checkpoint_handler_aliases_are_empty() {
        assert_eq!(CheckpointHandler::new().aliases(), &[] as &[&str]);
    }

    /// R6: when `session_id` is None, execute returns Err whose message
    /// mentions both `session_id` and `build_command_context` so the
    /// operator can trace the wiring bug. Mirrors the AGS-815 /fork
    /// and B17 /rename
    /// `execute_without_session_id_returns_err` precedent.
    #[test]
    fn execute_without_session_id_returns_err() {
        let (mut ctx, _rx) = make_ckpt_ctx(None);
        let h = CheckpointHandler::new();
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(
            res.is_err(),
            "CheckpointHandler::execute with None session_id must return \
             Err (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("session_id"),
            "Err message must mention 'session_id', got: {msg}"
        );
        assert!(
            msg.contains("build_command_context"),
            "Err message must mention 'build_command_context' to pin \
             the owning builder, got: {msg}"
        );
    }

    /// Branch 1: list + empty DB emits byte-exact
    /// `"\nNo checkpoints for this session.\n"` TextDelta.
    /// Redirects `dirs::data_dir()` to a tempdir via
    /// XDG_DATA_HOME+HOME mutation under an `env_lock()` guard so the
    /// CheckpointStore opens a fresh, empty sqlite DB.
    #[tokio::test]
    async fn execute_list_empty_emits_no_checkpoints_textdelta() {
        let _env_guard = env_lock().lock().expect("env_lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let sid = "test-b21-list-empty";
        let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
        let h = CheckpointHandler::new();
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx
            .recv()
            .await
            .expect("no-checkpoints TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(text, "\nNo checkpoints for this session.\n");
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nNo checkpoints for this \
                 session.\\n\"), got: {other:?}"
            ),
        }
    }

    /// Branch 2: list + non-empty DB emits a TextDelta starting with
    /// `"\nCheckpoints:\n"` and containing at least one per-entry
    /// line formatted as `"  turn {n} | {tool} | {path} | {ts}\n"`.
    /// Seeds the store via the public `snapshot()` API.
    #[tokio::test]
    async fn execute_list_non_empty_emits_formatted_textdelta() {
        let _env_guard = env_lock().lock().expect("env_lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let sid = "test-b21-list-nonempty";

        // Seed the real store at the same path the handler will open.
        // Mirror the handler's ckpt_path construction.
        let ckpt_path = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("archon")
            .join("checkpoints.db");
        let seed_file = tmp.path().join("seed.txt");
        std::fs::write(&seed_file, b"hello").expect("seed file write");
        {
            let store =
                archon_session::checkpoint::CheckpointStore::open(&ckpt_path)
                    .expect("open seed store");
            store
                .snapshot(
                    sid,
                    seed_file.to_str().expect("utf8"),
                    1,
                    "Edit",
                )
                .expect("seed snapshot");
        }

        let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
        let h = CheckpointHandler::new();
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        let ev = rx
            .recv()
            .await
            .expect("formatted TextDelta must be emitted");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert!(
                    text.starts_with("\nCheckpoints:\n"),
                    "non-empty list output must start with \
                     \"\\nCheckpoints:\\n\", got: {text:?}"
                );
                assert!(
                    text.contains("  turn 1 | Edit | "),
                    "non-empty list output must contain per-entry format \
                     prefix '  turn 1 | Edit | ', got: {text:?}"
                );
                let expected_path = seed_file.to_string_lossy();
                assert!(
                    text.contains(&*expected_path),
                    "non-empty list output must contain seeded file_path \
                     '{expected_path}', got: {text:?}"
                );
                assert!(
                    text.ends_with('\n'),
                    "non-empty list output must end with a trailing \
                     newline, got: {text:?}"
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta(..), got: {other:?}"
            ),
        }
    }

    /// Branch 4: `restore` with empty trailing path emits the
    /// byte-exact usage error.
    #[test]
    fn execute_restore_usage_error_on_empty_path() {
        // No env mutation needed — the empty-path branch short-circuits
        // BEFORE CheckpointStore::open, so dirs::data_dir() is never
        // consulted. Does not contend for env_lock().
        let sid = "test-b21-restore-empty";
        let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
        let h = CheckpointHandler::new();
        let res = h.execute(&mut ctx, &["restore".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");

        // Drain via try_recv — no env lock held, so we cannot await
        // (another test might be holding env_lock + awaiting). Use
        // blocking_recv-equivalent via a brief await inside a tokio
        // runtime isn't needed since this is a sync #[test], so use
        // `rx.try_recv()` directly.
        let ev = rx
            .try_recv()
            .expect("usage error must be emitted synchronously");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(msg, "Usage: /checkpoint restore <file_path>");
            }
            other => panic!(
                "expected TuiEvent::Error(\"Usage: /checkpoint restore \
                 <file_path>\"), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (list/empty success path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/checkpoint` with
    /// `CheckpointHandler::new()`; `Dispatcher::dispatch(&mut ctx,
    /// "/checkpoint list")` routes through the real alias+primary
    /// pipeline and surfaces the byte-exact no-checkpoints TextDelta.
    /// Uses the same XDG_DATA_HOME/HOME override scheme +
    /// `env_lock()` serialization as
    /// `execute_list_empty_emits_no_checkpoints_textdelta`.
    #[tokio::test]
    async fn dispatcher_routes_slash_checkpoint_list_with_session_emits_textdelta(
    ) {
        let _env_guard = env_lock().lock().expect("env_lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary(
            "checkpoint",
            Arc::new(CheckpointHandler::new()),
        );
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let sid = "test-b21-dispatch-list";
        let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
        let res = dispatcher.dispatch(&mut ctx, "/checkpoint list");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for list/empty, got: \
             {res:?}"
        );

        let ev = rx
            .recv()
            .await
            .expect("no-checkpoints TextDelta must be emitted via dispatcher");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(text, "\nNo checkpoints for this session.\n");
            }
            other => panic!(
                "expected TuiEvent::TextDelta(\"\\nNo checkpoints for this \
                 session.\\n\"), got: {other:?}"
            ),
        }
    }

    /// Dispatcher-integration test (error-surfacing path). Narrow
    /// `RegistryBuilder::new()` wires ONLY `/checkpoint` with
    /// `CheckpointHandler::new()`, dispatches `"/checkpoint list"`
    /// with `session_id: None`, and asserts that `Dispatcher::dispatch`
    /// surfaces the handler's Err (dispatcher.rs:110 forwards
    /// `handler.execute(..)` verbatim — it does NOT swallow
    /// handler-origin Errs). Mirrors B17 /rename precedent.
    #[test]
    fn dispatcher_routes_slash_checkpoint_without_session_returns_err() {
        let mut builder = RegistryBuilder::new();
        builder.insert_primary(
            "checkpoint",
            Arc::new(CheckpointHandler::new()),
        );
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, _rx) = make_ckpt_ctx(None);
        let res = dispatcher.dispatch(&mut ctx, "/checkpoint list");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must surface handler Err when \
             session_id is None, got: {res:?}"
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
