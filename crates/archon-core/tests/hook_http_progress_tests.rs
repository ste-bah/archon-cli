use archon_core::hooks::{HookConfig, HookOutcome, execute_http_hook};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn server(stall: bool, delay_headers: bool, slow_body: bool) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        if delay_headers {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        if stall {
            tokio::time::sleep(Duration::from_secs(10)).await;
        } else {
            let (count, interval) = if slow_body { (24, 250) } else { (60, 100) };
            for _ in 0..count {
                if stream.write_all(b"1\r\n \r\n").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(interval)).await;
            }
            let body = br#"{"outcome":"success","additional_context":"stream completed"}"#;
            let chunk = format!(
                "{:x}\r\n{}\r\n0\r\n\r\n",
                body.len(),
                String::from_utf8_lossy(body)
            );
            let _ = stream.write_all(chunk.as_bytes()).await;
        }
    });
    (format!("http://{address}/hook"), Server(task))
}

async fn run(stall: bool, delay_headers: bool, slow_body: bool) -> archon_core::hooks::HookResult {
    let (url, _server) = server(stall, delay_headers, slow_body).await;
    let config: HookConfig = serde_json::from_value(serde_json::json!({
        "type": "http", "command": url, "timeout": if stall { 1 } else { 5 },
        "on_failure": "allow"
    }))
    .unwrap();
    let client = archon_core::hooks::HookHttpTransport::new();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        execute_http_hook(
            &config,
            &serde_json::json!({"event":"PostToolUse"}),
            &client,
        ),
    )
    .await
    .expect("HTTP hang guard");
    if !stall && result.outcome == HookOutcome::Success {
        assert!(
            start.elapsed() > Duration::from_secs(5),
            "fixture did not exceed the configured window"
        );
    }
    result
}

#[tokio::test]
async fn streaming_body_has_no_total_timeout() {
    let result = run(false, false, false).await;
    assert_eq!(
        result.additional_context.as_deref(),
        Some("stream completed"),
        "{result:?}"
    );
}

#[tokio::test]
async fn response_wait_has_only_configured_no_progress_window() {
    let result = run(false, true, false).await;
    assert_eq!(
        result.additional_context.as_deref(),
        Some("stream completed"),
        "{result:?}"
    );
}

#[tokio::test]
async fn allowing_body_stall_is_an_explicit_no_progress_failure() {
    let result = run(true, false, false).await;
    assert_eq!(result.outcome, HookOutcome::NonBlockingError, "{result:?}");
    assert!(result.reason.unwrap_or_default().contains("no progress"));
}

#[tokio::test]
async fn slower_chunks_still_have_no_total_timeout() {
    let result = run(false, false, true).await;
    assert_eq!(
        result.additional_context.as_deref(),
        Some("stream completed"),
        "{result:?}"
    );
}
