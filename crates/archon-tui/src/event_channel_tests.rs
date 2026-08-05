use super::*;
use serial_test::serial;

fn progress(id: &str) -> TuiEvent {
    TuiEvent::VideoIngestProgress(crate::events::VideoIngestProgressEvent {
        video_id: id.into(),
        segment_count: 1,
        latest_text: String::new(),
        status: "processing".into(),
    })
}

fn reset_pending_metric() {
    crate::observability::reset_tui_drain_stall_state_for_tests();
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn nominal_capacity_is_a_hard_queue_bound() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(progress("a")).unwrap();
    tx.send(progress("b")).unwrap();
    let overflow = tx.send(progress("c"));

    assert!(overflow.is_err(), "full queue accepted another event");
    assert!(rx.len() <= 2, "queue exceeded its declared capacity");
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn async_send_waits_for_capacity_without_losing_payload() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("a")).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(progress("b")).await });
    tokio::task::yield_now().await;
    assert!(
        !waiting.is_finished(),
        "full queue did not apply backpressure"
    );

    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "a")
    );
    waiting
        .await
        .expect("sender task")
        .expect("waiting send should succeed");
    assert!(
        crate::observability::tui_event_blocked_send_duration_ns() > 0,
        "blocked send duration was not recorded"
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "b")
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn coalesced_text_frames_stay_bounded() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let chunk = "x".repeat(MAX_COALESCED_CONTENT_BYTES);
    tx.send(TuiEvent::TextDelta(chunk)).unwrap();
    tx.send(TuiEvent::TextDelta("y".into())).unwrap();

    let queue = rx.inner.queue.lock().expect("queue lock");
    assert_eq!(queue.len(), 2);
    assert!(queue.iter().all(|event| match event {
        TuiEvent::TextDelta(text) => text.len() <= MAX_COALESCED_CONTENT_BYTES,
        _ => true,
    }));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn receiver_close_wakes_blocked_async_sender() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("full")).unwrap();
    let blocked = tokio::spawn(async move { tx.send_async(progress("waiting")).await });
    tokio::task::yield_now().await;
    assert!(!blocked.is_finished());

    drop(rx);

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), blocked)
        .await
        .expect("blocked sender should wake")
        .expect("sender task")
        .expect_err("closed receiver should reject waiting event");
    assert!(matches!(error.0, TuiEvent::VideoIngestProgress(event) if event.video_id == "waiting"));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn payload_bearing_video_progress_is_lossless_under_pressure() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("a")).unwrap();
    let waiting = tokio::spawn(async move {
        tx.send_async(progress("b")).await?;
        tx.send_async(progress("c")).await
    });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "a"
    ));
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "b"
    ));
    waiting.await.expect("sender task").expect("waiting sends");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "c"
    ));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn payload_events_wait_for_capacity_without_loss() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(progress("a")).unwrap();
    tx.send(progress("b")).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(progress("c")).await });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "a"
    ));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "b"
    ));
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "c"
    ));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn state_and_payload_progress_preserve_order_under_backpressure() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(progress("old")).unwrap();
    tx.send(progress("new")).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(TuiEvent::Done).await });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "old"
    ));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "new"
    ));
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn bounded_channel_backpressures_lossless_state() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(TuiEvent::Done).await });

    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn resize_is_preserved_as_state_event() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("progress")).unwrap();
    let waiting = tokio::spawn(async move {
        tx.send_async(TuiEvent::Resize {
            cols: 120,
            rows: 40,
        })
        .await
    });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "progress"
    ));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::Resize {
            cols: 120,
            rows: 40
        })
    ));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn adjacent_text_deltas_coalesce_without_losing_bytes() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::TextDelta("hello ".into())).unwrap();
    tx.send(TuiEvent::TextDelta("世界".into())).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "hello 世界"
    ));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn full_queue_backpressures_without_shedding_text_delta() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(TuiEvent::TextDelta("first".into())).unwrap();
    tx.send(progress("ephemeral")).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(TuiEvent::Done).await });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "first"
    ));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "ephemeral"
    ));
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn text_coalescing_preserves_state_boundaries_and_exact_order() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(TuiEvent::TextDelta("a".into())).unwrap();
    tx.send(TuiEvent::TextDelta("b".into())).unwrap();
    tx.send(TuiEvent::GenerationStarted).unwrap();
    let waiting = tokio::spawn(async move {
        tx.send_async(TuiEvent::TextDelta("c".into())).await?;
        tx.send_async(TuiEvent::TextDelta("d".into())).await
    });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "ab"
    ));
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    waiting.await.expect("sender task").expect("waiting sends");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "cd"
    ));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn text_burst_reconstructs_exact_utf8_bytes() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(4);
    let chunks = ["α", "-", "世界", "\n", "final"];
    for chunk in chunks {
        tx.send(TuiEvent::TextDelta(chunk.into())).unwrap();
    }

    let Some(TuiEvent::TextDelta(text)) = rx.recv().await else {
        panic!("expected coalesced text delta");
    };
    assert_eq!(text, chunks.concat());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn transient_thinking_is_framed_coalesced_and_bounded() {
    let oversized = format!("{}界", "x".repeat(MAX_COALESCED_CONTENT_BYTES));
    let frames = bounded_content_events(TuiEvent::TransientThinkingDelta(oversized.clone()));
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|event| {
        matches!(event, TuiEvent::TransientThinkingDelta(text) if text.len() <= MAX_COALESCED_CONTENT_BYTES)
    }));
    let reconstructed = frames
        .iter()
        .map(|event| match event {
            TuiEvent::TransientThinkingDelta(text) => text.as_str(),
            _ => unreachable!("transient frame type"),
        })
        .collect::<String>();
    assert_eq!(reconstructed, oversized);

    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::TransientThinkingDelta("draft ".into()))
        .unwrap();
    tx.send(TuiEvent::TransientThinkingDelta("preview".into()))
        .unwrap();
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TransientThinkingDelta(text)) if text == "draft preview"
    ));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn adjacent_thinking_deltas_coalesce_without_losing_bytes() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    let chunks = ["reason ", "世界", "\n", "final"];
    for chunk in chunks {
        tx.send(TuiEvent::ThinkingDelta(chunk.into())).unwrap();
    }

    assert_eq!(tx.dropped_content(), 0);
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::ThinkingDelta(text)) if text == chunks.concat()
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
#[serial(tui_drain_metrics)]
fn last_sender_drop_cannot_race_receiver_wait() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    let inner = Arc::clone(&tx.inner);
    inner.pause_before_recv_wait.store(true, Ordering::Release);
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let receiver = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        result_tx
            .send(runtime.block_on(rx.recv()))
            .expect("send receiver result");
    });

    while !inner.recv_reached_wait.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    drop(tx);
    inner.pause_before_recv_wait.store(false, Ordering::Release);

    let result = result_rx.recv_timeout(std::time::Duration::from_millis(100));
    if result.is_err() {
        inner.notify.notify_one();
    }
    receiver.join().expect("receiver thread must not panic");
    assert!(
        result
            .expect("receiver must not hang after final sender drops")
            .is_none()
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn sender_rejects_events_after_receiver_closes() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(1);
    drop(rx);

    let error = tx
        .send(TuiEvent::TextDelta("lost".into()))
        .expect_err("closed receiver must reject send");
    assert!(matches!(error.0, TuiEvent::TextDelta(text) if text == "lost"));
    assert_eq!(
        crate::observability::tui_event_closed_send_failure_count(),
        1
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn concurrent_receiver_close_rejects_in_flight_send() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(1);
    let inner = Arc::clone(&tx.inner);
    inner.pause_before_send_lock.store(true, Ordering::Release);
    let sender = std::thread::spawn(move || {
        tx.send(TuiEvent::TextDelta("lost".into()))
            .map_err(Box::new)
    });

    while !inner.send_reached_lock.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    drop(rx);
    inner.pause_before_send_lock.store(false, Ordering::Release);

    let error = sender
        .join()
        .expect("sender thread must not panic")
        .expect_err("receiver closing during send must reject the event");
    assert!(matches!(error.0, TuiEvent::TextDelta(text) if text == "lost"));
}

// Depth is read from THIS channel, not from the process-wide
// `TUI_EVENT_PENDING` gauge. The gauge answers the same question for the render
// loop, which has exactly one channel -- but a test process has many,
// concurrently, all moving the same counter, so an exact assertion through it
// was an assertion about every other test's traffic too, and failed whenever
// anything unrelated shifted the schedule.
//
// These stay in the serial group for a DIFFERENT and real reason: sending on a
// channel also moves the process-wide send-failure and oversized-rejection
// totals, which `event_channel_payload_tests` asserts exact values on. Every
// test that sends needs the group; only tests that ASSERT DEPTH needed the
// global, and no longer do.
#[test]
#[serial(tui_drain_metrics)]
fn queue_depth_tracks_coalescing_and_dequeue() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::TextDelta("hello ".into())).unwrap();
    tx.send(TuiEvent::TextDelta("世界".into())).unwrap();
    assert_eq!(
        tx.queued_len(),
        1,
        "the second delta coalesces into the first"
    );

    rx.try_recv().unwrap();
    assert_eq!(tx.queued_len(), 0);
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn queue_depth_stays_bounded_under_backpressure() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(progress("old")).unwrap();
    tx.send(progress("new")).unwrap();
    let waiting = tokio::spawn({
        let tx = tx.clone();
        async move { tx.send_async(TuiEvent::Done).await }
    });
    tokio::task::yield_now().await;

    assert_eq!(
        tx.queued_len(),
        2,
        "a blocked sender must not exceed capacity"
    );
    rx.recv().await.expect("first event");
    waiting.await.expect("sender task").expect("waiting send");
    assert_eq!(tx.queued_len(), 2, "the waiting send takes the freed slot");
    rx.recv().await.expect("second event");
    rx.recv().await.expect("third event");
    assert_eq!(tx.queued_len(), 0);
}

#[test]
#[serial(tui_drain_metrics)]
fn receiver_close_clears_the_queue() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::GenerationStarted).unwrap();
    tx.send(TuiEvent::Done).unwrap();
    assert_eq!(tx.queued_len(), 2);

    drop(rx);
    assert_eq!(
        tx.queued_len(),
        0,
        "closing the receiver discards the queue"
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn full_synchronous_send_is_counted() {
    reset_pending_metric();
    let (tx, _rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();

    tx.send(TuiEvent::Done)
        .expect_err("full synchronous queue must reject send");

    assert_eq!(crate::observability::tui_event_full_send_failure_count(), 1);
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn text_delta_survives_backpressure_from_state_event() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::TextDelta("answer".into())).unwrap();
    let waiting = tokio::spawn(async move { tx.send_async(TuiEvent::Done).await });

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "answer"
    ));
    waiting.await.expect("sender task").expect("waiting send");
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}
