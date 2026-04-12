//! Tests for EventLog + EventBus + subscribe_events() (TASK-AGS-204).

use archon_core::tasks::events::{EventBus, EventLog};
use archon_core::tasks::models::{TaskEvent, TaskEventKind, TaskId};
use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio_stream::StreamExt;

fn make_event(task_id: TaskId, seq: u64, kind: TaskEventKind) -> TaskEvent {
    TaskEvent {
        task_id,
        seq,
        kind,
        payload: serde_json::json!({}),
        at: Utc::now(),
    }
}

#[tokio::test]
async fn test_events_monotonic_seq() {
    let tmp = TempDir::new().unwrap();
    let task_id = TaskId::new();
    let seq = Arc::new(AtomicU64::new(0));
    let log = EventLog::open(tmp.path(), task_id).await.unwrap();

    for _ in 0..1000 {
        let s = seq.fetch_add(1, Ordering::SeqCst);
        let evt = make_event(task_id, s, TaskEventKind::Progress);
        log.append(evt).await.unwrap();
    }

    // Replay and verify monotonic
    let events: Vec<TaskEvent> = log.replay(0).await.unwrap();
    assert_eq!(events.len(), 1000);
    for (i, evt) in events.iter().enumerate() {
        assert_eq!(evt.seq, i as u64);
    }
}

#[tokio::test]
async fn test_events_replay_from_seq() {
    let tmp = TempDir::new().unwrap();
    let task_id = TaskId::new();
    let log = EventLog::open(tmp.path(), task_id).await.unwrap();

    for i in 0..1000u64 {
        let evt = make_event(task_id, i, TaskEventKind::Progress);
        log.append(evt).await.unwrap();
    }

    let events = log.replay(500).await.unwrap();
    assert_eq!(events.len(), 500);
    assert_eq!(events[0].seq, 500);
    assert_eq!(events[499].seq, 999);
}

#[tokio::test]
async fn test_events_live_subscription_receives_new() {
    let bus = EventBus::new();
    let task_id = TaskId::new();

    let mut rx = bus.subscribe(task_id);

    let evt = make_event(task_id, 0, TaskEventKind::Started);
    bus.broadcast(task_id, evt.clone());

    let received = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        rx.next(),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(received.seq, 0);
    assert_eq!(received.kind, TaskEventKind::Started);
}

#[tokio::test]
async fn test_events_at_least_once_reconnect() {
    let tmp = TempDir::new().unwrap();
    let task_id = TaskId::new();
    let log = EventLog::open(tmp.path(), task_id).await.unwrap();

    // Write 10 events
    for i in 0..10u64 {
        let evt = make_event(task_id, i, TaskEventKind::Progress);
        log.append(evt).await.unwrap();
    }

    // "Disconnect" at seq 5, reconnect from seq 5
    let events = log.replay(5).await.unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].seq, 5);
}

#[tokio::test]
async fn test_events_single_writer_enforced() {
    let tmp = TempDir::new().unwrap();
    let task_id = TaskId::new();
    let log = Arc::new(EventLog::open(tmp.path(), task_id).await.unwrap());

    // Two concurrent appends — both should succeed without interleaving
    let log1 = log.clone();
    let log2 = log.clone();

    let h1 = tokio::spawn(async move {
        for i in 0..50u64 {
            let evt = make_event(task_id, i, TaskEventKind::Progress);
            log1.append(evt).await.unwrap();
        }
    });
    let h2 = tokio::spawn(async move {
        for i in 50..100u64 {
            let evt = make_event(task_id, i, TaskEventKind::Progress);
            log2.append(evt).await.unwrap();
        }
    });

    h1.await.unwrap();
    h2.await.unwrap();

    let events = log.replay(0).await.unwrap();
    assert_eq!(events.len(), 100);
}

#[tokio::test]
async fn test_events_replay_then_live_contiguous() {
    let tmp = TempDir::new().unwrap();
    let task_id = TaskId::new();
    let log = EventLog::open(tmp.path(), task_id).await.unwrap();
    let bus = EventBus::new();

    // Write 5 historical events
    for i in 0..5u64 {
        let evt = make_event(task_id, i, TaskEventKind::Progress);
        log.append(evt).await.unwrap();
    }

    // Replay from 0
    let historical = log.replay(0).await.unwrap();
    assert_eq!(historical.len(), 5);

    // Now subscribe for live
    let mut rx = bus.subscribe(task_id);
    let evt5 = make_event(task_id, 5, TaskEventKind::Finished);
    bus.broadcast(task_id, evt5);

    let live = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        rx.next(),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(live.seq, 5); // contiguous with replay end
}
