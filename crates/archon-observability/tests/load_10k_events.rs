//! TASK-AGS-OBS-914 load test.
//!
//! Emits 10,000 tracing events through `RedactionLayer` and asserts that the
//! wall-clock elapsed time stays below a generous threshold (5 seconds).  The
//! test also verifies that the captured output contains redaction markers,
//! proving the layer is actually exercising the regex path under load rather
//! than short-circuiting.
//!
//! Integration tests run in parallel, so we **never** install a global
//! subscriber.  `tracing::subscriber::with_default` scopes the subscriber to
//! the current thread only.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing_subscriber::layer::SubscriberExt;

/// Thread-safe in-memory writer that satisfies `std::io::Write + Send + 'static`.
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Load-test: 10,000 `tracing::info!` events through `RedactionLayer`.
///
/// * Pass criterion 1: wall-clock time < 5 seconds.
/// * Pass criterion 2: every emitted line contains the redaction marker,
///   confirming the regex engine is active under load.
#[test]
fn redaction_layer_load_10k_events_under_5s() {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = SharedWriter(Arc::clone(&sink));

    let layer = archon_observability::RedactionLayer::with_writer(writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let start = Instant::now();

    tracing::subscriber::with_default(subscriber, || {
        for i in 0..10_000 {
            // Include a synthetic secret-like value so the redaction regex
            // has real work to do on every event.
            tracing::info!(
                payload = format!("sk-abcdefghijklmnopqrst{:04}", i),
                "load event {i}"
            );
        }
    });

    let elapsed = start.elapsed();

    // Criterion 1: performance threshold.
    assert!(
        elapsed.as_secs() < 5,
        "10,000 redacted events took {:?}, expected < 5s",
        elapsed
    );

    // Criterion 2: output verification.
    let captured = String::from_utf8(sink.lock().unwrap().clone()).unwrap_or_default();

    // Count redaction markers; there should be at least as many as events
    // because every payload carries a secret-like prefix.
    let redaction_count = captured.matches("***REDACTED***").count();
    assert!(
        redaction_count >= 10_000,
        "expected >= 10_000 redaction markers, found {redaction_count}"
    );

    // Ensure none of the raw synthetic secrets leaked through.
    assert!(
        !captured.contains("sk-abcdefghijklmnopqrst"),
        "raw secret prefix leaked in output under load"
    );
}
