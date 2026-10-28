use super::*;
use crate::provider_fault_http_tests::{STATUSES, assert_transient, endpoint};

async fn check(reply: &'static [u8], hint: Option<u64>) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (url, peer) = endpoint(reply).await;
        let provider = VertexProvider::new(
            "AC-X-001".into(),
            "AC-X-001".into(),
            "AC-X-001".into(),
            "AC-X-001".into(),
            None,
        );
        let error = crate::provider_fault::defer_retry(provider.stream_at(
            LlmRequest {
                model: "AC-X-001".into(),
                ..Default::default()
            },
            &url,
            "AC-X-001",
        ))
        .await
        .unwrap_err();
        assert_transient(error, hint);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn cached_token_status_is_typed() {
    for &(reply, hint) in STATUSES {
        check(reply, hint).await;
    }
    check(b"", None).await;
}

#[tokio::test]
async fn streaming_body_drop_keeps_transport_class() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (url, peer) =
            endpoint(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n").await;
        let provider = VertexProvider::new(
            "AC-X-001".into(),
            "AC-X-001".into(),
            "AC-X-001".into(),
            "AC-X-001".into(),
            None,
        );
        let rx = provider
            .stream_at(
                LlmRequest {
                    model: "AC-X-001".into(),
                    ..Default::default()
                },
                &url,
                "AC-X-001",
            )
            .await
            .unwrap();
        let error = crate::completion_accumulator::collect_completion_response(rx)
            .await
            .unwrap_err();
        assert_transient(error, None);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
