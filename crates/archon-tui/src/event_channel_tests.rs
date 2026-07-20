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
async fn payload_bearing_video_progress_is_lossless_under_pressure() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("a")).unwrap();
    tx.send(progress("b")).unwrap();
    tx.send(progress("c")).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "a")
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "b")
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "c")
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn payload_events_may_exceed_capacity_without_loss() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(progress("a")).unwrap();
    tx.send(progress("b")).unwrap();
    tx.send(progress("c")).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "a")
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "b")
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "c")
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn state_and_payload_progress_preserve_order_above_capacity() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(progress("old")).unwrap();
    tx.send(progress("new")).unwrap();
    tx.send(TuiEvent::Done).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "old")
    );
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "new")
    );
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn bounded_channel_allows_lossless_state_above_capacity() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();
    tx.send(TuiEvent::Done).unwrap();

    assert_eq!(tx.dropped_state(), 0);
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn resize_is_preserved_as_state_event() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(progress("progress")).unwrap();
    tx.send(TuiEvent::Resize {
        cols: 120,
        rows: 40,
    })
    .unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::VideoIngestProgress(event)) if event.video_id == "progress"
    ));
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
async fn full_queue_never_sheds_text_delta() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(TuiEvent::TextDelta("first".into())).unwrap();
    tx.send(progress("ephemeral")).unwrap();
    tx.send(TuiEvent::Done).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "first"
    ));
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
    tx.send(TuiEvent::TextDelta("c".into())).unwrap();
    tx.send(TuiEvent::TextDelta("d".into())).unwrap();

    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "ab"
    ));
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
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
    let (tx, rx) = bounded_tui_event_channel_with_capacity(1);
    drop(rx);

    let error = tx
        .send(TuiEvent::TextDelta("lost".into()))
        .expect_err("closed receiver must reject send");
    assert!(matches!(error.0, TuiEvent::TextDelta(text) if text == "lost"));
}

#[test]
#[serial(tui_drain_metrics)]
fn concurrent_receiver_close_rejects_in_flight_send() {
    let (tx, rx) = bounded_tui_event_channel_with_capacity(1);
    let inner = Arc::clone(&tx.inner);
    inner.pause_before_send_lock.store(true, Ordering::Release);
    let sender = std::thread::spawn(move || tx.send(TuiEvent::TextDelta("lost".into())));

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

#[test]
#[serial(tui_drain_metrics)]
fn pending_metric_tracks_coalescing_and_dequeue() {
    reset_pending_metric();
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::TextDelta("hello ".into())).unwrap();
    tx.send(TuiEvent::TextDelta("世界".into())).unwrap();
    assert_eq!(crate::observability::tui_event_pending_count(), 1);

    rx.try_recv().unwrap();
    assert_eq!(crate::observability::tui_event_pending_count(), 0);
}

#[test]
#[serial(tui_drain_metrics)]
fn pending_metric_tracks_lossless_overflow() {
    reset_pending_metric();
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(progress("old")).unwrap();
    tx.send(progress("new")).unwrap();
    tx.send(TuiEvent::Done).unwrap();

    assert_eq!(crate::observability::tui_event_pending_count(), 3);
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    assert_eq!(crate::observability::tui_event_pending_count(), 0);
}

#[test]
#[serial(tui_drain_metrics)]
fn receiver_close_clears_pending_metric() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::GenerationStarted).unwrap();
    tx.send(TuiEvent::Done).unwrap();
    assert_eq!(crate::observability::tui_event_pending_count(), 2);

    drop(rx);
    assert_eq!(crate::observability::tui_event_pending_count(), 0);
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn text_delta_survives_when_lossless_events_exceed_capacity() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::TextDelta("answer".into())).unwrap();
    tx.send(TuiEvent::Done).unwrap();

    assert_eq!(tx.dropped_progress(), 0);
    assert_eq!(tx.dropped_state(), 0);
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::TextDelta(text)) if text == "answer"
    ));
    assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
}
