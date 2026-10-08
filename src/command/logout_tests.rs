use super::*;
// Used only by `cfg(unix)` tests in this module. See #136.
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use crate::command::dispatcher::Dispatcher;
#[cfg(unix)]
use crate::command::registry::CommandContext;
#[cfg(unix)]
use crate::command::registry::RegistryBuilder;

/// Process-wide lock so tests that mutate `HOME` do not race. It is the
/// command tests' shared `USER_DATA_ENV_LOCK`, so login and logout tests
/// serialise with each other and with the other command tests that take it. The
/// env_guard crate does not serialise arbitrary env vars across
/// threads, and setting `HOME` is inherently process-global.
/// `Mutex<()>` suffices — poisoning is tolerated (tests inside
/// `.lock()` may panic; the next test recovers). Mirrors the
/// AGS-815 / B20 / B22 env-mutation serialisation pattern.
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

/// Build a `CommandContext` with a freshly-created channel. /logout
/// adds no new CommandContext field, so every optional field is
/// `None` — mirroring `make_bug_ctx` (the other new-field-free
/// handler). No `auth_label` argument needed.
#[cfg(unix)]
fn make_logout_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn execute_no_credentials_emits_no_stored_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_logout_ctx();
    let h = LogoutHandler::new();
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
            ".archon/.credentials.json must not exist for the no-creds \
             branch, got: {}",
            cred_path.display()
        );

        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn execute_remove_success_emits_logged_out_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_logout_ctx();
    let h = LogoutHandler::new();
    let cred_path;
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");

        let _guard = EnvGuard::set("HOME", tmp.path());

        // Sanity: the file must exist pre-execute.
        assert!(
            cred_path.exists(),
            "cred_path must exist before execute, got: {}",
            cred_path.display()
        );

        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn execute_remove_failure_emits_error() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_logout_ctx();
    let h = LogoutHandler::new();
    {
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

        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok(), "execute must return Ok(()), got: {res:?}");
    }

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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn dispatcher_routes_slash_logout_no_creds_emits_textdelta() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_logout_ctx();
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        // No cred file.
        let _guard = EnvGuard::set("HOME", tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("logout", Arc::new(LogoutHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let res = dispatcher.dispatch(&mut ctx, "/logout");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );
    }

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
// `HOME` redirection is Unix-only. On Windows `dirs::home_dir()` resolves
// the profile through the shell known-folder API rather than an
// environment variable, so the override cannot be made hermetic there and
// the test would read (or worse, write) the real user profile.
#[cfg(unix)]
#[tokio::test]
async fn dispatcher_routes_slash_logout_removes_creds() {
    if crate::test_environment::isolated() {
        return;
    }
    let (mut ctx, mut rx) = make_logout_ctx();
    let cred_path;
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let archon_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).expect("create .archon dir");
        cred_path = archon_dir.join(".credentials.json");
        std::fs::write(&cred_path, "{}").expect("write credentials file");
        let _guard = EnvGuard::set("HOME", tmp.path());

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("logout", Arc::new(LogoutHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let res = dispatcher.dispatch(&mut ctx, "/logout");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok(()), got: {res:?}"
        );
    }

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
