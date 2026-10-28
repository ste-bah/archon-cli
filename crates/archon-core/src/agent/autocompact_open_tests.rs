//! Summary requests use the inference opening guard, including native HTTP clients.
use super::autocompact::generate_compaction_summary_with_usage;
use std::time::Duration;
use tokio::io::AsyncReadExt;

#[tokio::test(start_paused = true)]
async fn autocompact_open_without_headers_returns_operational_timeout() {
    tokio::time::resume();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        tokio::time::pause();
        let _ = socket.read(&mut [0]).await;
    });
    let provider =
        archon_llm::providers::OpenAiProvider::new("AC-X-001".into(), Some(url), "AC-X-001".into());
    let limit = Duration::from_secs(crate::config::DEFAULT_STREAM_IDLE_TIMEOUT_SECS);
    let messages = vec![serde_json::json!({"role":"user", "content":"AC-X-001"})];
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        limit * 2,
        archon_tools::provider_recovery::scope(archon_llm::provider_fault::defer_retry(
            generate_compaction_summary_with_usage(
                &provider,
                "AC-X-001",
                &messages,
                serde_json::Value::Null,
                32,
            ),
        )),
    )
    .await;
    server.abort();
    let error = result.expect("summary opening had no guard").unwrap_err();
    assert!(
        started.elapsed() >= limit,
        "connection closed before the opening guard ran"
    );
    assert!(
        matches!(error, super::autocompact::CompactionError::Provider(ref error) if error.is_operational()),
        "{error:?}"
    );
    preparation(false).await;
}

#[tokio::test(start_paused = true)]
async fn autocompact_preparation_observes_parent_cancellation() {
    preparation(true).await;
}

async fn preparation(cancelled: bool) {
    use std::sync::Arc;
    tokio::time::resume();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let cancel = tokio_util::sync::CancellationToken::new();
    let signal = cancel.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        tokio::time::pause();
        if cancelled {
            signal.cancel();
        }
        let _ = socket.read(&mut [0]).await;
    });
    let mut config = super::AgentConfig::default();
    config.context.context_window_override = Some(100_000);
    config.context.compact_threshold = 0.0;
    config.subagent_stream_idle_timeout_secs = 30;
    let runner = crate::subagent::runner::SubagentRunner::new(
        Arc::new(archon_llm::providers::OpenAiProvider::new(
            "AC-X-001".into(),
            Some(url),
            "AC-X-001".into(),
        )),
        "AC-X-001".into(),
        vec![],
        Arc::new(crate::dispatch::ToolRegistry::new()),
        archon_tools::tool::ToolContext {
            cancel_parent: Some(cancel),
            ..Default::default()
        },
        "AC-X-001".into(),
        100,
        10,
        Arc::new(config),
        Arc::new(archon_llm::identity::IdentityProvider::new(
            archon_llm::identity::IdentityMode::Clean,
            "AC-X-001".into(),
            String::new(),
            String::new(),
        )),
    );
    let result = tokio::time::timeout(
        Duration::from_secs(if cancelled { 1 } else { 60 }),
        archon_tools::provider_recovery::scope(archon_llm::provider_fault::defer_retry(
            runner.run("AC-X-001"),
        )),
    )
    .await;
    server.abort();
    let error = result
        .expect("preparation ignored cancellation")
        .unwrap_err();
    if cancelled {
        assert!(error.to_string().contains("cancelled"), "{error:?}");
    } else {
        assert!(
            matches!(error.downcast_ref::<archon_llm::provider::LlmError>(),
                Some(archon_llm::provider::LlmError::Transport(message))
                    if message == "connection timed out before response headers"),
            "summary outage did not reach the runner caller: {error:?}"
        );
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) {
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        socket.read_exact(&mut byte).await.unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let length = String::from_utf8(header)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    socket.read_exact(&mut vec![0; length]).await.unwrap();
}
