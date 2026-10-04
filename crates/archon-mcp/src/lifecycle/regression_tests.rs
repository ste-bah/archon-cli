//! Issue #281: exercise the actual protocol, lifecycle and output sinks.
use super::*;
use archon_observability::RedactionLayer;
use archon_tools::tool::{Tool, ToolContext};
use rmcp::service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use serde_json::{Value, json};
use std::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::prelude::*;

const VALUES: &[&str] = &[
    "eight888",
    "eight888-longer",
    "任意の値ですかな",
    "quoted\"value\\tail",
];
fn config() -> ServerConfig {
    serde_json::from_value(json!({
        "name": "regression", "command": "unused",
        "env": {"ordinary": VALUES[0], "overlap": VALUES[1], "short": "tiny"},
        "headers": {"X-Ordinary": VALUES[2], "X-Quoted": VALUES[3]}
    }))
    .expect("fixture config")
}
fn payload() -> String {
    format!("{} tiny", VALUES.join(" | "))
}
fn clean(text: &str) {
    for value in VALUES {
        assert!(!text.contains(value), "configured value leaked: {text}");
    }
    assert!(
        !text.contains("quoted\\\"value\\\\tail"),
        "escaped value leaked: {text}"
    );
    assert!(text.contains("tiny"), "short values should remain: {text}");
    assert!(
        !text.contains("[REDACTED]-longer"),
        "overlap was only partially masked: {text}"
    );
    assert!(text.contains("[REDACTED]"), "missing redaction: {text}");
}

struct Wire {
    tx: UnboundedSender<Value>,
    rx: UnboundedReceiver<RxJsonRpcMessage<RoleClient>>,
}
impl Transport<RoleClient> for Wire {
    type Error = std::io::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let tx = self.tx.clone();
        async move {
            tx.send(serde_json::to_value(item).map_err(std::io::Error::other)?)
                .map_err(std::io::Error::other)
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.rx.recv().await
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
async fn client(mode: &str) -> Result<McpClient, McpError> {
    let (tx, mut inbox) = unbounded_channel::<Value>();
    let (outbox, rx) = unbounded_channel();
    let mode = mode.to_string();
    tokio::spawn(async move {
        while let Some(message) = inbox.recv().await {
            let Some(id) = message.get("id") else {
                continue;
            };
            let method = message["method"].as_str().unwrap_or_default();
            let response = match method {
                "initialize" if mode == "init-error" => {
                    json!({"error": {"code": -32000, "message": payload()}})
                }
                "initialize" => {
                    json!({"result": {"protocolVersion": message["params"]["protocolVersion"], "capabilities": {"tools":{}}, "serverInfo": {"name":"fixture", "version":"0"}}})
                }
                "tools/list" if mode == "stall" => continue,
                "tools/list" if mode == "list-error" => {
                    json!({"error": {"code":-32000,"message":payload()}})
                }
                "tools/list" => {
                    json!({"result":{"tools":[{"name":"probe", "inputSchema":{"type":"object"}}]}})
                }
                "tools/call" if mode == "rpc-error" => {
                    json!({"error":{"code":-32000,"message":payload()}})
                }
                "tools/call" => json!({"result":{"isError":true,"content":[
                    {"type":"text","text":payload()},
                    {"type":"resource","resource":{"uri":format!("file:///{}", VALUES[0]),"text":payload()}},
                    {"type":"image","data":VALUES[1],"mimeType":VALUES[2]}
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
    McpClient::initialize(&config(), Wire { tx, rx }).await
}
async fn manager(mode: &str) -> McpServerManager {
    let manager = McpServerManager::new();
    manager.servers.write().await.insert(
        "regression".into(),
        ManagedServer {
            config: config(),
            state: ServerState::Ready,
            client: Some(Arc::new(
                client(mode).await.expect("fixture initialization"),
            )),
            restart_count: 0,
        },
    );
    manager
}
fn tool(client: Arc<McpClient>) -> crate::tool_bridge::McpTool {
    let def = serde_json::from_value(
        json!({"name":"probe", "input_schema":{}, "server_name":"regression"}),
    )
    .expect("fixture tool");
    crate::tool_bridge::McpTool::new("regression", def, client)
}
#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("capture lock"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn startup_logs_redact_configured_values() {
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    async {
        let mut cfg = config(); cfg.transport = payload();
        let _ = McpServerManager::new().start_all(vec![cfg]).await;
        let span = tracing::info_span!("eight888");
        let _entered = span.enter();
        tracing::warn!(target: "eight888-longer", diagnostic = %payload(), escaped = ?payload(), "MCP diagnostics");
    }.with_subscriber(subscriber).await;
    clean(&String::from_utf8(capture.0.lock().expect("capture").clone()).expect("log UTF-8"));
}
#[tokio::test]
async fn startup_errors_redact_configured_values() {
    let mut cfg = config();
    cfg.transport = payload();
    let errors = McpServerManager::new().start_all(vec![cfg]).await;
    clean(&format!("{:?} {}", errors, errors[0]));
}
#[tokio::test]
async fn initialization_errors_redact_configured_values() {
    let error = client("init-error")
        .await
        .err()
        .expect("rejected initialize");
    clean(&format!("{error:?} {error}"));
}
#[tokio::test]
async fn client_call_errors_redact_configured_values() {
    let client = client("rpc-error").await.expect("initialize");
    let error = client
        .call_tool("probe", None)
        .await
        .expect_err("RPC error");
    clean(&format!("{error:?} {error}"));
}
#[tokio::test]
async fn bridge_call_errors_redact_configured_values() {
    let result = tool(Arc::new(client("rpc-error").await.expect("initialize")))
        .execute(Value::Null, &ToolContext::default())
        .await;
    assert!(result.is_error);
    clean(&result.content);
}
#[tokio::test]
async fn client_error_content_redacts_all_content_fields() {
    let result = client("content-error")
        .await
        .expect("initialize")
        .call_tool("probe", None)
        .await
        .expect("tool result");
    assert!(result.is_error);
    clean(&serde_json::to_string(&result).expect("result JSON"));
}
#[tokio::test]
async fn bridge_error_content_redacts_configured_values() {
    let result = tool(Arc::new(client("content-error").await.expect("initialize")))
        .execute(Value::Null, &ToolContext::default())
        .await;
    assert!(result.is_error);
    clean(&result.content);
}
#[tokio::test]
async fn bridge_input_errors_redact_configured_values() {
    let result = tool(Arc::new(client("content-error").await.expect("initialize")))
        .execute(json!(payload()), &ToolContext::default())
        .await;
    assert!(result.is_error);
    clean(&result.content);
}
#[tokio::test]
async fn discovery_errors_and_logs_redact_configured_values() {
    let manager = manager("list-error").await;
    let error = manager
        .tools_for("regression")
        .await
        .err()
        .expect("list fails");
    clean(&error.to_string());
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    manager
        .build_mcp_tools()
        .with_subscriber(
            tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone())),
        )
        .await;
    clean(&String::from_utf8(capture.0.lock().expect("capture").clone()).expect("UTF-8"));
}
async fn discover(manager: &McpServerManager, path: u8) {
    match path {
        0 => {
            let _ = manager.build_mcp_tools().await;
        }
        1 => {
            let _ = manager.list_tools_for("regression").await;
        }
        _ => {
            let _ = manager.tools_for("regression").await;
        }
    }
}
async fn unlocked(path: u8) {
    let manager = manager("stall").await;
    let discovery = discover(&manager, path);
    tokio::pin!(discovery);
    tokio::select! { _ = &mut discovery => panic!("stall answered"), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), manager.servers.write())
            .await
            .is_ok(),
        "discovery holds the manager lock across await"
    );
}
async fn bounded(path: u8) {
    let empty = McpServerManager::new();
    assert!(empty.list_tools_for("absent").await.is_empty());
    assert!(empty.build_mcp_tools().await.is_empty());
    assert!(empty.tools_for("absent").await.is_err());
    let healthy = manager("healthy").await;
    assert_eq!(healthy.list_tools_for("regression").await.len(), 1);
    let manager = manager("stall").await;
    tokio::time::pause();
    assert!(
        tokio::time::timeout(Duration::from_secs(31), discover(&manager, path))
            .await
            .is_ok(),
        "stalled discovery has no deadline"
    );
}
#[tokio::test]
async fn build_discovery_releases_lock() {
    unlocked(0).await;
}
#[tokio::test]
async fn list_discovery_releases_lock() {
    unlocked(1).await;
}
#[tokio::test]
async fn build_discovery_has_idle_deadline() {
    bounded(0).await;
}
#[tokio::test]
async fn list_discovery_has_idle_deadline() {
    bounded(1).await;
}
#[tokio::test]
async fn tools_for_has_idle_deadline() {
    bounded(2).await;
}

// Progress must extend the idle deadline, but stopping progress must still end it.
#[tokio::test]
async fn progressing_discovery_then_stall_obeys_idle_only_budget() {
    let (tx, mut inbox) = unbounded_channel::<Value>();
    let (outbox, rx) = unbounded_channel();
    tokio::spawn(async move {
        let mut listings = 0;
        while let Some(message) = inbox.recv().await {
            let Some(id) = message.get("id") else {
                continue;
            };
            if message["method"] == "initialize" {
                let response = json!({"jsonrpc":"2.0", "id":id, "result":{
                    "protocolVersion":message["params"]["protocolVersion"], "capabilities":{"tools":{}},
                    "serverInfo":{"name":"progress", "version":"0"}}});
                let _ = outbox.send(serde_json::from_value(response).expect("fixture response"));
            } else if message["method"] == "tools/list" {
                let token = message["params"]["_meta"]["progressToken"].clone();
                for progress in 1..=4 {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let notification = json!({"jsonrpc":"2.0", "method":"notifications/progress", "params":{"progressToken":token,"progress":progress}});
                    let _ = outbox
                        .send(serde_json::from_value(notification).expect("fixture progress"));
                }
                listings += 1;
                if listings == 1 {
                    let response = json!({"jsonrpc":"2.0", "id":id, "result":{"tools":[]}});
                    let _ = outbox.send(serde_json::from_value(response).expect("fixture tools"));
                }
                // Second listing falls silent: deadline is 30s after last progress.
            }
        }
    });
    let client = McpClient::initialize(&config(), Wire { tx, rx })
        .await
        .expect("initialize");
    tokio::time::pause();
    let start = tokio::time::Instant::now();
    assert!(
        client.list_tools().await.is_ok(),
        "progressing discovery failed"
    );
    assert!(
        start.elapsed() >= Duration::from_secs(40),
        "fixture must exceed an idle interval in total"
    );
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(75), client.list_tools()).await;
    assert!(
        result.is_ok(),
        "discovery never times out after progress stops"
    );
    assert!(
        start.elapsed() >= Duration::from_secs(70),
        "discovery enforced a total rather than idle budget"
    );
    assert!(result.expect("bounded wait").is_err());
}

#[tokio::test]
async fn spawn_errors_redact_configured_executable_values() {
    let mut cfg = config();
    cfg.command = format!("/missing/{}", payload());
    let error = crate::transport::spawn_transport(&cfg)
        .err()
        .expect("spawn fails");
    clean(&error.to_string());
}

#[cfg(unix)]
#[tokio::test]
async fn child_stderr_is_logged_with_configured_value_redaction() {
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber =
        tracing_subscriber::registry().with(RedactionLayer::with_writer(capture.clone()));
    async {
        let mut cfg = config();
        cfg.command = "/bin/sh".into();
        cfg.args = vec![
            "-c".into(),
            "printf '%s\\n' \"$ordinary\" \"$overlap\" \"$short\" >&2; cat".into(),
        ];
        let _transport = crate::transport::spawn_transport(&cfg).expect("shell transport");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    .with_subscriber(subscriber)
    .await;
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("UTF-8");
    assert!(
        log.contains("MCP server stderr"),
        "child diagnostics bypass the redacted log sink: {log}"
    );
    clean(&log);
}
