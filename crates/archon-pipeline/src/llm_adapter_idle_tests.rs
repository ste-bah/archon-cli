//! Issue 356: every actual provider event, including reasoning and pings, renews.
use super::*;
use std::time::Duration;

async fn activity(event: StreamEvent) {
    let progress = archon_shell::progress::Progress::default();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let send = tokio::spawn(async move {
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_secs(7_000)).await;
            tx.send(event.clone()).await.unwrap();
        }
        tx.send(StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        })
        .await
        .unwrap();
    });
    let result = progress
        .bound(
            Duration::from_secs(7_200),
            collect_stream_into(rx, None, Some(progress.clone())),
        )
        .await;
    assert!(
        result.is_ok(),
        "a live stream must outlast the former total"
    );
    assert_eq!(
        result.unwrap().unwrap().stop_reason.as_deref(),
        Some("end_turn")
    );
    send.await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn issue356_provider_text_activity() {
    activity(StreamEvent::TextDelta {
        index: 0,
        text: "x".into(),
    })
    .await;
}
#[tokio::test(start_paused = true)]
async fn issue356_provider_reasoning_activity() {
    activity(StreamEvent::ThinkingDelta {
        index: 0,
        thinking: "x".into(),
    })
    .await;
}
#[tokio::test(start_paused = true)]
async fn issue356_provider_ping_activity() {
    activity(StreamEvent::Ping).await;
}
