//! OBS-905 LIFT smoke test.
//!
//! Written BEFORE the LIFT lands (dev-flow Gate 1: tests-written-first).
//! This file intentionally fails to compile until OBS-905 has added:
//!
//!   * `pub mod tracing;` to `crates/archon-observability/src/lib.rs`
//!   * the lifted contents of
//!     `crates/archon-tui/src/observability_tracing.rs` into
//!     `crates/archon-observability/src/tracing.rs`
//!   * `pub use` re-exports for the five symbols exercised below
//!
//! The test's purpose is to pin the post-LIFT public surface so a future
//! refactor cannot accidentally drop, privatise, or rename any of them
//! without a compile error at the external-crate boundary. Each assertion
//! is a *shape* check (does this symbol exist? does it return the expected
//! span / result type?) — the behavioural contract (secret redaction,
//! layer stacking) remains covered by the unit tests that lift alongside
//! the impl in `src/tracing.rs`.

// OBS-906 carve: RedactionLayer now lives in the `redaction` submodule. The
// remaining tracing-glue surface (init_tracing + span_*) stays in `tracing`.
// Both are also available at the crate root via `pub use`, but this smoke
// test pins the submodule paths explicitly so a future silent rearrangement
// triggers a compile error here instead of silently relocating the API.
use archon_observability::redaction::RedactionLayer;
use archon_observability::tracing::{
    init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch,
};
use tracing_subscriber::layer::SubscriberExt;

/// `init_tracing` must remain idempotent across repeated calls from test
/// binaries, mirroring the contract preserved by the lifted unit test
/// `init_tracing_is_idempotent`. Exercising it at the external-crate
/// boundary proves the `pub` + `pub use` chain is intact after the LIFT.
#[test]
fn init_tracing_is_reachable_and_idempotent() {
    let first = init_tracing(false, ::tracing::Level::INFO);
    let second = init_tracing(true, ::tracing::Level::DEBUG);
    assert!(first.is_ok(), "first init_tracing must succeed");
    assert!(second.is_ok(), "second init_tracing must be idempotent Ok");
}

/// All three span constructors must remain reachable as external-crate
/// symbols and must continue to carry their documented fields.
///
/// The `with_default` scope is load-bearing, not decorative: with NO
/// subscriber installed for the current thread, `info_span!` collapses to
/// `Span::none()` (a disabled span with empty metadata), and
/// `span.field(name)` would then return `None` regardless of what the
/// macro literal lists. Integration tests run in parallel threads by
/// default, so we cannot rely on another test having installed a global
/// subscriber first — each assertion must install its own. The captured
/// `Vec<u8>` sink is never inspected; we only care that the subscriber is
/// ACTIVE for the duration of the `span_*` calls so the resulting
/// `Span::field()` lookups return `Some`.
#[test]
fn span_constructors_reachable_with_fields() {
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(Vec::<u8>::new()));
    ::tracing::subscriber::with_default(subscriber, || {
        let turn = span_agent_turn("t-smoke");
        assert!(turn.field("task_id").is_some(), "agent.turn missing task_id");
        assert!(turn.field("turn_ms").is_some(), "agent.turn missing turn_ms");

        let slash = span_slash_dispatch("t-smoke", "/noop");
        assert!(slash.field("task_id").is_some(), "slash missing task_id");
        assert!(slash.field("command").is_some(), "slash missing command");

        let send = span_channel_send("t-smoke", "AgentDelta");
        assert!(send.field("task_id").is_some(), "channel.send missing task_id");
        assert!(send.field("event_kind").is_some(), "channel.send missing event_kind");
    });
}

/// `RedactionLayer::with_writer` must remain `pub` — this is the hook the
/// in-process tests (and any future external capture harness) use to
/// redirect emitted lines. The assertion is a type-only check: merely
/// constructing the layer against an in-memory sink proves reachability
/// without installing a global subscriber from within the smoke test.
#[test]
fn redaction_layer_public_constructor_reachable() {
    let sink: Vec<u8> = Vec::new();
    let _layer = RedactionLayer::with_writer(sink);
    // If we got here, the type + constructor are still `pub`. No panic path.
}
