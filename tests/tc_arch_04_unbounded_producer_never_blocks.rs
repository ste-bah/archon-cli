//! TC-ARCH-04: bounded producer backpressure preserves every event.

use archon_core::agent::AgentEvent;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_waits_for_capacity_without_event_loss() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(2);
    tx.send(AgentEvent::TextDelta("event-0".into()))
        .await
        .unwrap();
    tx.send(AgentEvent::TextDelta("event-1".into()))
        .await
        .unwrap();

    let blocked =
        tokio::spawn(async move { tx.send(AgentEvent::TextDelta("event-2".into())).await });
    tokio::task::yield_now().await;
    assert!(
        !blocked.is_finished(),
        "full bounded channel did not backpressure producer"
    );

    let first = rx.recv().await.expect("first event");
    blocked
        .await
        .expect("producer task")
        .expect("blocked event should resume");
    let second = rx.recv().await.expect("second event");
    let third = rx.recv().await.expect("third event");

    let text = [first, second, third]
        .into_iter()
        .map(|event| match event {
            AgentEvent::TextDelta(text) => text,
            _ => panic!("unexpected event"),
        })
        .collect::<Vec<_>>();
    assert_eq!(text, ["event-0", "event-1", "event-2"]);
}
