//! OBS-901-WIRE smoke test — exercises `init_tracing_file` end-to-end.
//!
//! Written BEFORE the wiring lands (dev-flow Gate 1). Pins the post-wire
//! behaviour that closes the pre-LIFT dead-wire where production session
//! logs bypassed `RedactionLayer`:
//!
//!   1. The call creates `{session_id}.log` in `log_dir` (directory is
//!      created on demand).
//!   2. On Unix, the log file has mode 0600 — same secret-file posture
//!      that pre-LIFT `init_logging` guaranteed.
//!   3. Events emitted after `init_tracing_file` returns go through
//!      `RedactionLayer` — the `***REDACTED***` marker appears on the
//!      line carrying a sk- OpenAI key, and the raw secret bytes do NOT.
//!
//! Single test per integration binary because `tracing` can only be
//! global-installed once per process (same constraint documented in
//! `crates/archon-core/tests/logging_tests.rs`). Separate binaries do
//! NOT share this state — each `tests/*.rs` file is its own process.
//!
//! The unit tests in `src/redaction.rs` already cover the full 9-secret-
//! shape matrix via `with_default` (thread-local subscriber). This file
//! complements them with a single end-to-end assertion on the
//! production global-install path.

use std::fs;
use std::io::Read;

#[test]
fn init_tracing_file_redacts_secrets_and_writes_session_log() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let log_dir = tmp.path().to_path_buf();
    let session_id = "obs901-wire-smoke";

    // `init_tracing_file` installs the global subscriber. Hold the guard
    // so the non-blocking worker is flushed on drop (below) — without
    // the drop the file contents can race the assertion on the writer's
    // internal channel.
    let guard = archon_observability::init_tracing_file(session_id, "info", &log_dir)
        .expect("init_tracing_file must succeed on a writable temp dir");

    // Emit an event carrying a real-shape OpenAI secret. The redaction
    // layer MUST scrub it before the line lands in the file.
    ::tracing::info!(
        api_key = "sk-abcdefghijklmnopqrst0000",
        "secret event for OBS-901-WIRE smoke"
    );

    // Flush the non-blocking writer by dropping its WorkerGuard.
    drop(guard);

    let log_path = log_dir.join(format!("{session_id}.log"));
    assert!(
        log_path.exists(),
        "log file not created by init_tracing_file: {log_path:?}"
    );

    let mut contents = String::new();
    fs::File::open(&log_path)
        .expect("open session log")
        .read_to_string(&mut contents)
        .expect("read session log");

    assert!(
        !contents.is_empty(),
        "session log is empty; expected at least the emitted event"
    );
    assert!(
        contents.contains("***REDACTED***"),
        "expected REDACTED marker in session log; got:\n{contents}"
    );
    assert!(
        !contents.contains("sk-abcdefghijklmnopqrst0000"),
        "raw OpenAI secret leaked into session log — dead-wire NOT closed:\n{contents}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&log_path).expect("stat session log");
        let mode = meta.mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "session log perms must be 0600 (secret posture preserved from pre-LIFT init_logging); got {mode:o}"
        );
    }
}
