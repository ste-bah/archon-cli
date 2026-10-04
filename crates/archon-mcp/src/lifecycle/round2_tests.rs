//! Hostile probes for Issue #281, round 2.
use super::*;

const WORDS: &str =
    "unexpected token at line 3; check secret password authorization credentials api_key and retry";
const OWN: &str = "round-two-own-value";
const OTHER: &str = "round-two-other-value";

fn round2_config() -> ServerConfig {
    serde_json::from_value(json!({
        "name": OWN, "command": "unused",
        "env": {"ACCESS_TOKEN": OWN},
    }))
    .expect("fixture")
}

async fn round2_client(mode: &str) -> McpClient {
    let (tx, mut inbox) = unbounded_channel::<Value>();
    let (outbox, rx) = unbounded_channel();
    let mode = mode.to_string();
    tokio::spawn(async move {
        while let Some(message) = inbox.recv().await {
            let Some(id) = message.get("id") else {
                continue;
            };
            let body = format!("{WORDS}; {OWN}; {OTHER}; quoted\\\"round2\\\\value");
            let response = match message["method"].as_str().unwrap_or_default() {
                "initialize" => json!({"result": {
                    "protocolVersion": message["params"]["protocolVersion"],
                    "capabilities": {"tools":{}},
                    "serverInfo": {"name":"fixture", "version":"0"}
                }}),
                "tools/call" if mode == "rpc-error" => {
                    json!({"error": {"code": -32000, "message":body}})
                }
                "tools/list" => json!({"error": {"code": -32000, "message":body}}),
                "tools/call" => json!({"result": {"isError": true, "content": [
                    {"type":"text", "text":body},
                    {"type":"resource", "resource":{"uri":format!("file:///{body}"), "text":body}},
                    {"type":"image", "data":body, "mimeType":body}
                ]}}),
                _ => continue,
            };
            let mut response = response;
            response["jsonrpc"] = json!("2.0");
            response["id"] = id.clone();
            if let Ok(response) = serde_json::from_value(response) {
                let _ = outbox.send(response);
            }
        }
    });
    let mut cfg = round2_config();
    cfg.env
        .insert("PRIVATE_KEY".into(), "quoted\"round2\\value".into());
    McpClient::initialize(&cfg, Wire { tx, rx })
        .await
        .expect("fixture initialization")
}

fn preserved(text: &str) {
    assert!(
        text.contains(WORDS),
        "ordinary diagnostic words damaged: {text}"
    );
    assert!(!text.contains(OWN), "own credential leaked: {text}");
    assert!(
        !text.contains("quoted\\\"round2\\\\value"),
        "escaped own credential leaked: {text}"
    );
    assert!(
        text.contains(OTHER),
        "another server's value masked: {text}"
    );
}

fn register_other() {
    let mut other = round2_config();
    other.env = [("API_KEY".into(), OTHER.into())].into();
    other.configured_secrets().register();
}

#[tokio::test]
async fn round2_rpc_error_preserves_words_and_server_scope() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    register_other();
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    let client = round2_client("rpc-error").with_subscriber(subscriber).await;
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("log");
    assert!(
        !log.contains(OWN) && log.contains("[REDACTED]"),
        "initialize did not register its secrets: {log}"
    );
    preserved(
        &client
            .call_tool("probe", None)
            .await
            .expect_err("RPC rejection")
            .to_string(),
    );
}

#[tokio::test]
async fn round2_rpc_error_does_not_mask_another_servers_values() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    register_other();
    let client = round2_client("rpc-error").await;
    let text = client
        .call_tool("probe", None)
        .await
        .expect_err("RPC rejection")
        .to_string();
    assert!(
        text.contains(OTHER),
        "another server's value masked: {text}"
    );
    assert!(!text.contains(OWN), "own credential leaked: {text}");
}

#[tokio::test]
async fn round2_discovery_error_preserves_words_and_server_scope() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    register_other();
    let client = round2_client("rpc-error").await;
    preserved(
        &client
            .list_tools()
            .await
            .expect_err("discovery rejection")
            .to_string(),
    );
}

#[tokio::test]
async fn round2_tool_content_preserves_words_and_server_scope() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    register_other();
    let client = Arc::new(round2_client("content-error").await);
    let result = client.call_tool("probe", None).await.expect("result");
    for content in result.content {
        match content {
            crate::types::ToolContent::Text { text } => preserved(&text),
            crate::types::ToolContent::Resource { uri, text } => {
                preserved(&uri);
                preserved(&text.expect("text"));
            }
            crate::types::ToolContent::Image { data, mime_type } => {
                preserved(&data);
                preserved(&mime_type);
            }
        }
    }
    let result = tool(client)
        .execute(Value::Null, &ToolContext::default())
        .await;
    assert!(result.is_error);
    preserved(&result.content);
}

#[test]
fn round2_configuration_values_survive_logs() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    let mut cfg = round2_config();
    cfg.env.extend([
        ("MODE".into(), "production".into()),
        ("PYTHONPATH".into(), "/opt/round2/python".into()),
    ]);
    cfg.headers = Some([("Content-Type".into(), "application/json".into())].into());
    cfg.configured_secrets().register();
    let text = "production application/json /opt/round2/python";
    assert_eq!(archon_observability::redaction::redact_text(text), text);
}

#[test]
fn round2_bare_and_encoded_header_values_are_redacted() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    let cfg: ServerConfig = serde_json::from_value(json!({
        "name": "encoded", "headers": {
            "aUtHoRiZaTiOn": "Bearer round2-bare/token+value",
            "Proxy-Authorization": "Basic cm91bmQyLWJhc2lj",
            "Cookie": "session=round2-cookie+value"
        }, "env": {"PRIVATE_KEY": "round2/url+value with space"}
    }))
    .expect("fixture");
    cfg.configured_secrets().register();
    for value in [
        "round2-bare/token+value",
        "cm91bmQyLWJhc2lj",
        "round2-bare%2Ftoken%2Bvalue",
        "Bearer%20round2-bare%2Ftoken%2Bvalue",
        "round2%2Furl%2Bvalue%20with%20space",
        "session%3Dround2-cookie%2Bvalue",
    ] {
        assert!(
            !cfg.configured_secrets().text(value).contains(value),
            "returned value leaked: {value}"
        );
        assert!(
            !archon_observability::redaction::redact_text(value).contains(value),
            "logged value leaked: {value}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn round2_non_utf8_stderr_does_not_kill_child() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("marker");
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    async {
        let mut cfg = round2_config();
        cfg.command = "/bin/sh".into();
        cfg.args = vec!["-c".into(), r#"printf 'before\n\377\n' >&2; sleep 0.1; printf 'afterline\n' >&2 || exit; printf marker > "$1"; cat"#.into(), "probe".into(), marker.to_string_lossy().into_owned()];
        let _transport = crate::transport::spawn_transport(&cfg).expect("shell");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !marker.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(marker.exists(), "stderr reader closed the pipe before the marker write");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }.with_subscriber(subscriber).await;
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("UTF-8 log");
    assert!(
        !log.contains(OWN),
        "spawn did not register its secrets: {log}"
    );
    assert!(
        log.contains("before") && log.contains("afterline"),
        "reader did not drain: {log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn round2_stderr_line_is_capped_and_following_line_is_drained() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    async {
        let mut cfg = round2_config();
        cfg.command = "/bin/sh".into();
        cfg.args = vec![
            "-c".into(),
            "printf '%20000s\\n' x >&2; printf 'following-line\\n' >&2; printf 'unterminated-tail' >&2; exec 2>&-; cat".into(),
        ];
        let _transport = crate::transport::spawn_transport(&cfg).expect("shell");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    .with_subscriber(subscriber)
    .await;
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("UTF-8 log");
    assert!(log.contains("following-line"), "following line not drained");
    assert!(
        log.contains("unterminated-tail"),
        "EOF fragment not drained: {log}"
    );
    assert!(
        log.len() < 9000,
        "one logged line was not capped: {} bytes",
        log.len()
    );
}

struct RecoveringReader(bool);
impl tokio::io::AsyncRead for RecoveringReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.0 {
            self.0 = true;
            return std::task::Poll::Ready(Err(std::io::Error::other("transient read failure")));
        }
        if buf.remaining() > 0 && self.0 {
            buf.put_slice(b"after-read-error\n");
            // The EOF is supplied by Take's byte budget below.
        }
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn round2_stderr_retries_read_errors_until_eof() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    use tokio::io::AsyncReadExt;
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    crate::transport::drain_stderr(RecoveringReader(false).take(17), "recovering".into())
        .with_subscriber(subscriber)
        .await;
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("log");
    assert!(
        log.contains("after-read-error"),
        "reader stopped on a read error: {log}"
    );
}
