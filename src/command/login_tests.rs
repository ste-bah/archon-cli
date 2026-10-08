use super::*;
use std::sync::Arc;

use crate::command::dispatcher::Dispatcher;
use crate::command::registry::{CommandContext, RegistryBuilder};

/// Process-wide lock so tests that mutate `HOME` do not race. It is the
/// command tests' shared `USER_DATA_ENV_LOCK`, so login and logout tests
/// serialise with each other and with the other command tests that take it. The
/// env_guard crate does not serialise arbitrary env vars across
/// threads, and setting `HOME` is inherently process-global.
/// `Mutex<()>` suffices — poisoning is tolerated (tests inside
/// `.lock()` may panic; the next test recovers). Mirrors the
/// AGS-815 / B20 env-mutation serialisation pattern.
// Used only by `cfg(unix)` tests in this module. See #136.
#[cfg(unix)]
use crate::command::USER_DATA_ENV_LOCK as ENV_LOCK;

/// RAII guard that overrides an env var for the lifetime of a
/// single test body and restores the prior value on drop.
/// Not re-entrant. Acquire `ENV_LOCK` FIRST.
#[cfg(unix)]
struct EnvGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

#[cfg(unix)]
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

#[cfg(unix)]
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
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_auth_label_opt(auth_label)
        .build()
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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn execute_authenticated_branch_emits_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_login_ctx(Some("anthropic-api-key".to_string()));
    let h = LoginHandler::new();
    let cred_path;
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");

        let _guard = EnvGuard::set("HOME", tmp.path());

        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
        other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
    }
}

/// Not-authenticated branch integration test. Override `HOME` to a
/// tempdir WITHOUT `.archon/.credentials.json`; assert the handler
/// emits a single byte-exact TextDelta containing the 4-step
/// OAuth-instructions block.
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn execute_not_authenticated_branch_emits_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_login_ctx(Some("api-key".to_string()));
    let h = LoginHandler::new();
    {
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
            ".archon/.credentials.json must not exist for the not-auth \
             branch, got: {}",
            cred_path.display()
        );

        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
        other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
    }
}

/// Dispatcher-integration test (authenticated path). Narrow
/// `RegistryBuilder::new()` wires ONLY `/login` with
/// `LoginHandler::new()`, then
/// `Dispatcher::dispatch(&mut ctx, "/login")` routes through the
/// real alias+primary pipeline. Uses `HOME`-override + cred-file
/// trick to select the authenticated branch deterministically.
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn dispatcher_routes_slash_login_with_auth_label_emits_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_login_ctx(Some("oauth".to_string()));
    {
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

        let res = dispatcher.dispatch(&mut ctx, "/login");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );
    }

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
        other => panic!("expected TuiEvent::TextDelta(..), got: {other:?}"),
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
        msg.contains("auth_label") && msg.contains("build_command_context"),
        "Err message must mention both 'auth_label' and \
         'build_command_context', got: {msg}"
    );
}
