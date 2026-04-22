//! OBS-907 end-to-end proof that `init_tracing(json=true, _)` actually
//! produces JSON-shaped lines on stderr — the one seam the existing test
//! suite cannot cover in-process because `init_tracing` wires the layer
//! to `std::io::stderr()` directly (no writer injection point) and only
//! installs once per process.
//!
//! Design: self-re-exec. The test binary acts as two programs guarded
//! by the `OBS907_CHILD_JSON` environment variable:
//!
//!   * CHILD (env var set) — calls `init_tracing(true, Level::INFO)`,
//!     emits a single tracing event carrying a real-shape OpenAI secret,
//!     flushes via process exit.
//!   * PARENT (env var unset) — spawns the child via
//!     `std::process::Command::new(std::env::current_exe())`, captures
//!     its stderr as bytes, and asserts the output is JSON-shaped AND
//!     the raw secret does not appear.
//!
//! Why self-re-exec instead of `cargo run --example`:
//!   - `cargo run` during `cargo test` pollutes the build state and
//!     breaks `--offline`.
//!   - `current_exe()` is the test binary itself, already built, free.
//!   - Child/parent share the same code path — no separate binary, no
//!     separate Cargo entry, no additional dev-dep.
//!
//! Why this test exists (and why the existing ones aren't enough):
//!   - `tracing_smoke.rs::init_tracing_is_reachable_and_idempotent`
//!     proves `init_tracing(true, _)` returns Ok but does NOT assert
//!     anything about the bytes emitted to stderr.
//!   - `redaction.rs::redaction_layer_json_layout_is_parseable` proves
//!     `RedactionLayer::with_writer_and_format(_, true)` emits valid
//!     JSON, but uses a capturing sink and bypasses `init_tracing`
//!     entirely — so a future refactor that broke the
//!     `init_tracing -> stderr_with_format -> with_writer_and_format`
//!     wire would still pass both existing tests.
//!
//! This file closes that seam with the ONE assertion that catches it:
//! "when I call `init_tracing(true, ...)` and emit an event, stderr
//! contains a JSON-shaped line with the secret redacted".

use std::process::{Command, Stdio};

/// Env-var switch. Presence = "I am the child, do the emit."
const CHILD_MARKER: &str = "OBS907_CHILD_JSON";

/// Shape-matching strings the child's stderr MUST contain when
/// `init_tracing(true, _)` is honestly wired to the JSON code path.
/// These are the same three shape markers asserted by the in-process
/// redaction unit test (`redaction_layer_json_layout_is_parseable`),
/// so a regression in either the `stderr_with_format` constructor OR
/// the `with_writer_and_format` JSON branch would fail here.
const JSON_SHAPE_LEVEL: &str = "\"level\":\"INFO\"";
const JSON_SHAPE_FIELDS: &str = "\"fields\":{";

/// Real-shape OpenAI secret emitted by the child. Must NEVER appear in
/// captured stderr — if it does, `init_tracing(true, _)` bypassed the
/// redaction layer.
const RAW_SECRET: &str = "sk-obs907e2esecret000000000";

#[test]
fn init_tracing_json_true_produces_redacted_json_on_stderr() {
    if std::env::var(CHILD_MARKER).is_ok() {
        // ========== CHILD BRANCH ==========
        // Install the production tracing stack with json=true, emit one
        // secret-bearing event, exit. All subsequent assertions run in
        // the parent against our stderr.
        archon_observability::init_tracing(true, ::tracing::Level::INFO)
            .expect("child: init_tracing(true, INFO) must succeed");
        ::tracing::info!(api_key = RAW_SECRET, "obs907 json e2e event");
        // Exit explicitly so the tracing emitter flushes through the
        // normal drop path. No panic, no abort — just a clean exit that
        // the parent's `.wait()` observes.
        std::process::exit(0);
    }

    // ========== PARENT BRANCH ==========
    let exe = std::env::current_exe().expect("parent: current_exe");

    // Filter is a noop because this test itself is the only test that
    // matches, but we still pass `--nocapture` so the child doesn't
    // buffer output through cargo's runtime filter and our capture sees
    // exactly the tracing-emitted bytes.
    let output = Command::new(&exe)
        .env(CHILD_MARKER, "1")
        // Pin the filter level inside the child so a hostile CI
        // environment with RUST_LOG=error (or any other setting that
        // filters out INFO) cannot cause a false-negative wire-break
        // diagnosis. init_tracing's EnvFilter reads RUST_LOG via
        // try_from_default_env; "info" is the lowest level that still
        // emits the single INFO event this test fires.
        .env("RUST_LOG", "info")
        .arg("--nocapture")
        .arg("--exact")
        .arg("init_tracing_json_true_produces_redacted_json_on_stderr")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("parent: spawn child test-binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Sanity: child exited cleanly (exit code 0 from std::process::exit).
    // A panic in `init_tracing(true, _)` or the tracing emit would break
    // this before we ever got to shape assertions.
    assert!(
        output.status.success(),
        "child process did not exit cleanly: status={:?}\nstderr={}",
        output.status,
        stderr
    );

    // Shape (A): the JSON level field must be present. If
    // `stderr_with_format(true)` silently flipped to json=false, this
    // fails — the pretty format is `[INFO target]` with no quotes.
    assert!(
        stderr.contains(JSON_SHAPE_LEVEL),
        "expected JSON-shape level marker {JSON_SHAPE_LEVEL:?} in child stderr; got:\n{stderr}"
    );

    // Shape (B): the JSON fields object must be opened. Pretty format
    // has no `"fields":{` substring so this catches the same wire break.
    assert!(
        stderr.contains(JSON_SHAPE_FIELDS),
        "expected JSON-shape fields marker {JSON_SHAPE_FIELDS:?} in child stderr; got:\n{stderr}"
    );

    // Redaction contract: the raw secret MUST be scrubbed by
    // `RedactionLayer`. A parallel fmt::layer in the stack would leak
    // it here because stderr is shared.
    assert!(
        !stderr.contains(RAW_SECRET),
        "raw secret {RAW_SECRET:?} leaked into child stderr — dead-wire regression:\n{stderr}"
    );

    // Positive marker: the tracing event message must appear in the
    // output so we know the child actually emitted (and we are not just
    // reading an empty stderr that happens to not contain the secret).
    assert!(
        stderr.contains("obs907 json e2e event"),
        "child stderr missing the emitted message body; got:\n{stderr}"
    );
}
