use super::*;
use std::sync::Arc;

use crate::command::dispatcher::Dispatcher;
use crate::command::registry::{CommandContext, RegistryBuilder};

/// RAII guard that sets `XDG_DATA_HOME` + `HOME` to the supplied
/// tempdir on construction and restores the prior values on drop.
/// The caller must hold `crate::command::USER_DATA_ENV_LOCK` for the
/// guard's lifetime to prevent cross-test environment races. Mirrors
/// B17 /rename `EnvGuard` precedent at
/// src/command/rename.rs:248.
struct EnvGuard {
    prev_xdg: Option<std::ffi::OsString>,
    prev_home: Option<std::ffi::OsString>,
    prev_data: Option<std::ffi::OsString>,
}
impl EnvGuard {
    fn set(tmp: &std::path::Path) -> Self {
        let g = Self {
            prev_xdg: std::env::var_os("XDG_DATA_HOME"),
            prev_home: std::env::var_os("HOME"),
            prev_data: std::env::var_os("ARCHON_DATA_DIR"),
        };
        // SAFETY: env mutation is protected by the command-wide
        // `USER_DATA_ENV_LOCK` acquired by every command test that mutates
        // XDG_DATA_HOME/HOME.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", tmp);
            std::env::set_var("HOME", tmp);
            // XDG/HOME do not steer `dirs::data_dir()` on Windows; this is
            // what actually redirects the store off the real user profile.
            std::env::set_var("ARCHON_DATA_DIR", tmp);
        }
        g
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: see `EnvGuard::set`. Shared lock still held by caller.
        unsafe {
            match self.prev_xdg.take() {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match self.prev_data.take() {
                Some(v) => std::env::set_var("ARCHON_DATA_DIR", v),
                None => std::env::remove_var("ARCHON_DATA_DIR"),
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
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
    if crate::test_environment::isolated() {
        return;
    }
    let sid = "test-b21-list-empty";
    let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
    let h = CheckpointHandler::new();
    {
        let _env_guard = crate::command::USER_DATA_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
    if crate::test_environment::isolated() {
        return;
    }
    let sid = "test-b21-list-nonempty";
    let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
    let h = CheckpointHandler::new();
    let seed_file;
    {
        let _env_guard = crate::command::USER_DATA_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        // Seed the real store at the same path the handler will open.
        // Mirror the handler's ckpt_path construction.
        let ckpt_path = archon_session::background::archon_data_dir().join("checkpoints.db");
        seed_file = tmp.path().join("seed.txt");
        std::fs::write(&seed_file, b"hello").expect("seed file write");
        {
            let store = archon_session::checkpoint::CheckpointStore::open(&ckpt_path)
                .expect("open seed store");
            store
                .snapshot(sid, seed_file.to_str().expect("utf8"), 1, "Edit")
                .expect("seed snapshot");
        }

        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
        other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
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
async fn dispatcher_routes_slash_checkpoint_list_with_session_emits_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let sid = "test-b21-dispatch-list";
    let (mut ctx, mut rx) = make_ckpt_ctx(Some(sid.to_string()));
    {
        let _env_guard = crate::command::USER_DATA_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("checkpoint", Arc::new(CheckpointHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let res = dispatcher.dispatch(&mut ctx, "/checkpoint list");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()) for list/empty, got: \
             {res:?}"
        );
    }

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
    builder.insert_primary("checkpoint", Arc::new(CheckpointHandler::new()));
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
        msg.contains("session_id") && msg.contains("build_command_context"),
        "Err message must mention both 'session_id' and \
         'build_command_context', got: {msg}"
    );
}
