//! Process-level fault seams for the offline lifecycle test
//! (`tests/workflow_decomposition_offline_lifecycle.rs`), which drives the
//! real binary and so cannot use `cfg(test)`.
//!
//! Every seam reads its variable only in a build with debug assertions (the
//! `dev` and `test` profiles that `cargo test` builds the binary with). A
//! release build compiles each function to its production behaviour and
//! never reads the variables. Production values stay compile-time or come
//! from the catalog; no seam changes a stored limit, digest or identity
//! other than the one it names.
//!
//! - `ARCHON_TEST_BINARY_REVISION`: the build id a fixed decomposition
//!   launch, resume and resume dry run record as the running build, in place
//!   of the compile-time `ARCHON_GIT_HASH`. A test launches under one value
//!   and resumes under the real one to prove binary-drift handling.
//! - `ARCHON_TEST_HOST_COMMAND_TIMEOUT_ONCE=<commandId>`: the first attempt
//!   of that host command in this process gets a zero no-progress window, so
//!   the supervisor's real timeout ending (tree teardown, `timed_out`) and
//!   the retry policy run. The catalog, the limits fingerprint and every
//!   record keep the catalog's window.
//! - `ARCHON_TEST_HOLD_CALL=<call id prefix | commandId>` with
//!   `ARCHON_TEST_HOLD_DIR=<dir>`: the first matching host call in this
//!   process writes its call id to `<dir>/held`, then waits, before run
//!   control is polled for it, until `<dir>/release` exists. A test pauses or
//!   kills the run while it waits, which is the boundary between the previous
//!   call's end and this call's start.

/// The seam variables, recognised (not warned about) only in a build that
/// reads them; a release build warns that they are unrecognised, which they are.
pub(crate) fn known_env_vars() -> &'static [&'static str] {
    if cfg!(debug_assertions) {
        &[
            "ARCHON_TEST_BINARY_REVISION",
            "ARCHON_TEST_HOST_COMMAND_TIMEOUT_ONCE",
            "ARCHON_TEST_HOLD_CALL",
            "ARCHON_TEST_HOLD_DIR",
        ]
    } else {
        &[]
    }
}

/// The build id this process records as the running build.
pub(crate) fn binary_revision() -> String {
    #[cfg(debug_assertions)]
    if let Some(revision) = std::env::var("ARCHON_TEST_BINARY_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return revision;
    }
    env!("ARCHON_GIT_HASH").to_string()
}

/// The supervisor's no-progress window for one attempt of `command_id`.
pub(crate) fn host_command_window(command_id: &str, timeout_secs: u64) -> std::time::Duration {
    #[cfg(debug_assertions)]
    if std::env::var("ARCHON_TEST_HOST_COMMAND_TIMEOUT_ONCE")
        .ok()
        .as_deref()
        == Some(command_id)
        && first_match(&TIMEOUT_FIRED)
    {
        return std::time::Duration::ZERO;
    }
    let _ = command_id;
    std::time::Duration::from_secs(timeout_secs)
}

/// Waits at the boundary before `call_id` when the hold seam names it.
pub(crate) async fn hold_before_call(call_id: &str, command_id: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let (Ok(name), Ok(dir)) = (
            std::env::var("ARCHON_TEST_HOLD_CALL"),
            std::env::var("ARCHON_TEST_HOLD_DIR"),
        ) else {
            return;
        };
        let named =
            !name.is_empty() && (call_id.starts_with(&name) || command_id == Some(name.as_str()));
        if !named || !first_match(&HOLD_FIRED) {
            return;
        }
        let dir = std::path::PathBuf::from(dir);
        // Renamed into place, so a reader never sees a partial call id.
        let written = std::fs::write(dir.join("held.partial"), call_id)
            .and_then(|()| std::fs::rename(dir.join("held.partial"), dir.join("held")));
        if let Err(error) = written {
            panic!("test hold seam cannot write {}: {error}", dir.display());
        }
        while !dir.join("release").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (call_id, command_id);
}

#[cfg(debug_assertions)]
static TIMEOUT_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(debug_assertions)]
static HOLD_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True exactly once per process for `fired`.
#[cfg(debug_assertions)]
fn first_match(fired: &std::sync::atomic::AtomicBool) -> bool {
    !fired.swap(true, std::sync::atomic::Ordering::SeqCst)
}
