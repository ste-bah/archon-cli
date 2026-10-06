//! A real HTTP response and SSE decoder, never a synthetic progress recorder.
use super::*;
use archon_workflow::WorkflowLlmClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) struct Transport {
    frames: tokio::sync::mpsc::Sender<(String, tokio::sync::oneshot::Sender<()>)>,
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Transport {
    pub(crate) async fn ready(&mut self) {
        self.ready.take().unwrap().await.unwrap();
        settle().await;
    }
    pub(crate) async fn frame(&self, frame: &str) {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        self.frames.send((frame.into(), tx)).await.unwrap();
        // No automatic time jump while a real socket write wakes the server.
        let started = std::time::Instant::now();
        loop {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(30),
                "fixture socket write stalled"
            );
            match rx.try_recv() {
                Ok(()) => break,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("fixture response writer closed: {error}"),
            }
        }
        settle().await;
    }
}
pub(crate) async fn settle() {
    // Keep paused time from auto-advancing while real socket readiness travels
    // through reqwest, the decoder, provider observer and workflow adapters.
    for _ in 0..500 {
        tokio::task::yield_now().await;
    }
}
pub(crate) const PING: &str = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
pub(crate) fn answer(text: &str) -> String {
    format!(
        "event: content_block_delta\ndata: {}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":10}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}})
    )
}
pub(crate) async fn client_for(
    backstop: Option<u64>,
    headers: bool,
    local: bool,
) -> (Arc<dyn WorkflowLlmClient>, Transport) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (frames, mut rx) =
        tokio::sync::mpsc::channel::<(String, tokio::sync::oneshot::Sender<()>)>(1);
    let (ready_tx, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let header = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&buffer[..n]);
                assert!(request.len() <= 2 * 1024 * 1024);
                if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let len: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap_or("0")
                        .trim()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + len {
                        break header;
                    }
                }
            };
            if header.starts_with("get ") {
                // The real local adapter discovers model metadata at construction.
                let body = r#"{"data":[{"id":"fixture-model","max_model_len":32000}]}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                continue;
            }
            assert!(header.starts_with("post "));
            break socket;
        };
        if headers {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            socket
                .write_all(format!("{:x}\r\n{PING}\r\n", PING.len()).as_bytes())
                .await
                .unwrap();
        }
        ready_tx.send(()).unwrap();
        while let Some((frame, ack)) = rx.recv().await {
            if socket
                .write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes())
                .await
                .is_err()
            {
                return;
            }
            let _ = ack.send(());
        }
    });
    let provider = if local && backstop.is_some() {
        Arc::new(LocalProvider::new(
            endpoint.trim_end_matches("/v1/messages").into(),
            "fixture-model".into(),
            backstop.unwrap(),
            false,
        )) as Arc<dyn LlmProvider>
    } else if let Some(backstop) = backstop {
        use archon_llm::{
            auth::AuthProvider,
            identity::{IdentityMode, IdentityProvider},
            types::Secret,
        };
        Arc::new(AnthropicProvider::new(AnthropicClient::with_read_backstop(
            AuthProvider::ApiKey(Secret::new("fixture-key".into())),
            IdentityProvider::new(
                IdentityMode::Clean,
                "fixture".into(),
                "device".into(),
                String::new(),
            ),
            Some(endpoint),
            backstop,
        ))) as Arc<dyn LlmProvider>
    } else {
        let mut config = ArchonConfig::default();
        if local {
            config.llm.provider = "local".into();
            config.llm.local.base_url = endpoint.trim_end_matches("/v1/messages").into();
            config.llm.local.model = "fixture-model".into();
            config.llm.local.timeout_secs = 300;
            config.llm.local.pull_if_missing = false;
        } else {
            config.llm.provider = "anthropic".into();
            config.api.base_url = Some(endpoint);
        }
        config.identity.mode = "clean".into();
        let mut env = archon_core::env_vars::load_env_vars_from(&Default::default());
        env.anthropic_api_key = Some("fixture-key".into());
        build_configured_llm_provider_with_policy(
            &config,
            &env,
            "transport-regression",
            crate::command::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
        )
        .await
        .unwrap()
    };
    let adapter = archon_pipeline::llm_adapter::ProviderLlmAdapter::new(provider);
    (
        crate::command::pipeline_workflow_llm::PipelineWorkflowLlmClient::arc(Arc::new(adapter)),
        Transport {
            frames,
            ready: Some(ready),
            server,
        },
    )
}

pub(crate) fn openai_delta(text: &str, complete: bool) -> String {
    let stop = if complete {
        serde_json::json!("stop")
    } else {
        serde_json::Value::Null
    };
    let mut frame = format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":stop}]})
    );
    if complete {
        frame.push_str("data: [DONE]\n\n");
    }
    frame
}
