//! OBS-906 carve smoke test.
//!
//! Written BEFORE the carve lands (dev-flow Gate 1). This file pins two
//! independent surfaces so the OBS-906 refactor cannot silently break
//! downstream callers OR future intra-crate code that reaches into the
//! dedicated `redaction` module:
//!
//!   1. **External surface**: `archon_observability::RedactionLayer` must
//!      still resolve via the crate-root `pub use`. Existing call sites
//!      (e.g. the archon-tui re-export shim) continue to compile after the
//!      carve with zero edits.
//!
//!   2. **Internal surface**: `archon_observability::redaction::{RedactionLayer,
//!      REDACTED}` must resolve at the new submodule path. OBS-907 and any
//!      future work that needs direct redaction access (e.g. a standalone
//!      `redact()` helper exposed to tests) depends on this path being
//!      stable post-carve.
//!
//! The tests below are shape checks, not behavioural — the full secret
//! matrix + parallel-sinks-not-filters regression guard already lives in
//! the unit tests inside `src/redaction.rs` (which move alongside the
//! impl during the carve).

use tracing_subscriber::layer::SubscriberExt;

/// Crate-root re-export check: OBS-905's commitment was that
/// `archon_observability::RedactionLayer` is the stable external handle.
/// The OBS-906 carve must not silently drop that re-export.
#[test]
fn crate_root_redaction_layer_reexport_stable() {
    let sink: Vec<u8> = Vec::new();
    let _layer = archon_observability::RedactionLayer::with_writer(sink);
    // If this compiles and constructs without panic, the crate-root
    // re-export survived the carve.
}

/// Submodule path check: after the carve, `redaction` becomes a first-class
/// module and callers that want a stable handle *specifically* for redaction
/// concerns should be able to reach it directly. Pinning this path keeps
/// OBS-907's gate-walk of JSON output from having to know which module
/// currently owns `RedactionLayer`.
#[test]
fn redaction_submodule_path_reachable() {
    let sink: Vec<u8> = Vec::new();
    let _layer = archon_observability::redaction::RedactionLayer::with_writer(sink);
    // Likewise: compile + construct = path stable.
}

/// End-to-end redaction guard at the external-crate boundary. The unit
/// tests in `src/redaction.rs` exercise the full secret matrix under
/// `with_default`; this integration test proves the same layer instance
/// reached via the public re-export correctly redacts a representative
/// secret shape when wired into a standalone subscriber.
///
/// Race-fix parity with OBS-905: parallel test threads cannot rely on a
/// previously-installed global subscriber. We install our own on the
/// current thread via `with_default`.
#[test]
fn redaction_layer_scrubs_secret_via_public_surface() {
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink_clone = std::sync::Arc::clone(&sink);

    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let layer = archon_observability::RedactionLayer::with_writer(SharedWriter(sink_clone));
    let subscriber = tracing_subscriber::registry().with(layer);

    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::info!(
            payload = "sk-abcdefghijklmnopqrst0000",
            "post-carve smoke"
        );
    });

    let captured = String::from_utf8(sink.lock().unwrap().clone()).unwrap_or_default();
    assert!(
        captured.contains("***REDACTED***"),
        "expected redaction marker in emitted line, got: {captured:?}"
    );
    assert!(
        !captured.contains("sk-abcdefghijklmnopqrst0000"),
        "raw secret leaked post-carve: {captured:?}"
    );
}
