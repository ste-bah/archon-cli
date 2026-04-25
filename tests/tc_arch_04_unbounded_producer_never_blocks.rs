//! TC-ARCH-04 (REQ-FOR-D3): Producer never blocks when consumer is paused.
//!
//! Create a real unbounded AgentEvent channel, pause receiver for 2s,
//! emit 10_000 events back-to-back, resume receiver. Assert:
//! - Every `.send()` returns in <1ms
//! - Receiver drains all 10_000 events after resume
//! - Zero event loss

use std::time::Instant;

use archon_core::agent::AgentEvent;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_never_blocks_during_receiver_pause() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    let event_count = 10_000usize;

    // Spawn a receiver that pauses for 2s before draining.
    let receiver = tokio::spawn(async move {
        // Simulate consumer pause (e.g. TUI render backpressure)
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let mut received = 0usize;
        while let Some(_event) = rx.recv().await {
            received += 1;
            if received >= event_count {
                break;
            }
        }
        received
    });

    // Producer: emit 10_000 events back-to-back. Each send must be <1ms.
    let mut max_send_us = 0u128;
    let mut violations = Vec::new();

    for i in 0..event_count {
        let event = AgentEvent::TextDelta(format!("event-{i}"));
        let start = Instant::now();
        let result = tx.send(event);
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "send {i} failed — receiver dropped");

        let us = elapsed.as_micros();
        if us > max_send_us {
            max_send_us = us;
        }
        // 1ms = 1000us
        if elapsed.as_millis() >= 1 {
            violations.push((i, elapsed));
        }
    }

    // Drop sender so receiver loop exits after draining
    drop(tx);

    // Wait for receiver to drain
    let received = tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
        .await
        .expect("receiver timed out")
        .expect("receiver panicked");

    // Assertions
    assert!(
        violations.is_empty(),
        "TC-ARCH-04: {}/{event_count} sends took >= 1ms. Max: {max_send_us}us. \
         Violations: {:?}",
        violations.len(),
        &violations[..violations.len().min(10)]
    );

    assert_eq!(
        received, event_count,
        "TC-ARCH-04: receiver drained {received}/{event_count} events — data loss detected"
    );
}
