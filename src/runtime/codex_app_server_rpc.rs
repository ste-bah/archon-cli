//! Minimal Codex app-server JSON-RPC transport.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use archon_core::config::CodexProviderConfig;
use archon_llm::provider::LlmError;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message,
};

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, LlmError>>>>>;
type WebSocketSink = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[derive(Debug, Clone)]
pub(crate) struct CodexNotification {
    pub(crate) method: String,
    pub(crate) params: Value,
}

pub(crate) struct CodexAppServerRpcClient {
    writer: Arc<Mutex<RpcWriter>>,
    pending: PendingMap,
    next_id: AtomicU64,
    _child: Option<Child>,
}

enum RpcWriter {
    Stdio(ChildStdin),
    WebSocket(WebSocketSink),
}

impl CodexAppServerRpcClient {
    pub(crate) async fn connect(
        config: &CodexProviderConfig,
    ) -> Result<(Self, mpsc::Receiver<CodexNotification>), LlmError> {
        match normalize_transport(&config.app_server_transport).as_str() {
            "stdio" => connect_stdio(config).await,
            "websocket" | "ws" => connect_websocket(config).await,
            other => Err(LlmError::Unsupported(format!(
                "unsupported Codex app-server transport `{other}`"
            ))),
        }
    }

    pub(crate) async fn initialize(&self, timeout_ms: u64) -> Result<Value, LlmError> {
        self.request(
            "initialize",
            serde_json::json!({
                "clientInfo": {
                    "name": "archon",
                    "title": "Archon",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            }),
            timeout_ms,
        )
        .await
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, LlmError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let frame = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write_frame(&frame).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(Duration::from_millis(timeout_ms.max(100)), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(LlmError::Http(format!(
                "Codex app-server request `{method}` channel closed"
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(LlmError::Http(format!(
                    "Codex app-server request `{method}` timed out"
                )))
            }
        }
    }

    async fn write_frame(&self, frame: &Value) -> Result<(), LlmError> {
        let line = serde_json::to_string(frame).map_err(|e| LlmError::Serialize(e.to_string()))?;
        let mut writer = self.writer.lock().await;
        match &mut *writer {
            RpcWriter::Stdio(stdin) => {
                stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))?;
                stdin
                    .write_all(b"\n")
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))
            }
            RpcWriter::WebSocket(ws) => ws
                .send(Message::Text(line.into()))
                .await
                .map_err(|e| LlmError::Http(e.to_string())),
        }
    }
}

impl Drop for CodexAppServerRpcClient {
    fn drop(&mut self) {
        if let Some(child) = &mut self._child {
            let _ = child.start_kill();
        }
    }
}

async fn connect_stdio(
    config: &CodexProviderConfig,
) -> Result<(CodexAppServerRpcClient, mpsc::Receiver<CodexNotification>), LlmError> {
    let mut child = Command::new(&config.app_server_command)
        .args(&config.app_server_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| LlmError::Http(format!("failed to spawn Codex app-server: {e}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| LlmError::Http("Codex app-server stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LlmError::Http("Codex app-server stdout unavailable".into()))?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(drain_stderr(stderr));
    }
    let writer = Arc::new(Mutex::new(RpcWriter::Stdio(stdin)));
    let pending = PendingMap::default();
    let (notify_tx, notify_rx) = mpsc::channel(256);
    tokio::spawn(read_stdio(
        stdout,
        Arc::clone(&pending),
        Arc::clone(&writer),
        notify_tx,
    ));
    Ok((client(writer, pending, Some(child)), notify_rx))
}

async fn connect_websocket(
    config: &CodexProviderConfig,
) -> Result<(CodexAppServerRpcClient, mpsc::Receiver<CodexNotification>), LlmError> {
    let url = config
        .app_server_url
        .as_deref()
        .ok_or_else(|| LlmError::Http("Codex app-server WebSocket URL is not configured".into()))?;
    let (socket, _) = connect_async(websocket_url(url))
        .await
        .map_err(|e| LlmError::Http(format!("Codex app-server WebSocket connect failed: {e}")))?;
    let (sink, stream) = socket.split();
    let writer = Arc::new(Mutex::new(RpcWriter::WebSocket(sink)));
    let pending = PendingMap::default();
    let (notify_tx, notify_rx) = mpsc::channel(256);
    tokio::spawn(read_websocket(
        stream,
        Arc::clone(&pending),
        Arc::clone(&writer),
        notify_tx,
    ));
    Ok((client(writer, pending, None), notify_rx))
}

fn client(
    writer: Arc<Mutex<RpcWriter>>,
    pending: PendingMap,
    child: Option<Child>,
) -> CodexAppServerRpcClient {
    CodexAppServerRpcClient {
        writer,
        pending,
        next_id: AtomicU64::new(1),
        _child: child,
    }
}

async fn read_stdio(
    stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    writer: Arc<Mutex<RpcWriter>>,
    notify_tx: mpsc::Sender<CodexNotification>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        handle_frame(&line, &pending, &writer, &notify_tx).await;
    }
    reject_all(pending, "Codex app-server stdio closed").await;
}

async fn read_websocket(
    mut stream: futures_util::stream::SplitStream<
        WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    >,
    pending: PendingMap,
    writer: Arc<Mutex<RpcWriter>>,
    notify_tx: mpsc::Sender<CodexNotification>,
) {
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { break };
        match message {
            Message::Text(text) => handle_frame(&text, &pending, &writer, &notify_tx).await,
            Message::Binary(bytes) => {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    handle_frame(text, &pending, &writer, &notify_tx).await;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    reject_all(pending, "Codex app-server WebSocket closed").await;
}

async fn handle_frame(
    line: &str,
    pending: &PendingMap,
    writer: &Arc<Mutex<RpcWriter>>,
    notify_tx: &mpsc::Sender<CodexNotification>,
) {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return;
    };
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        if value.get("method").and_then(Value::as_str).is_some() {
            reply_unsupported_request(id, writer).await;
            return;
        }
        let result = if let Some(error) = value.get("error") {
            Err(LlmError::Http(format!(
                "Codex app-server RPC error: {error}"
            )))
        } else {
            Ok(value.get("result").cloned().unwrap_or(Value::Null))
        };
        if let Some(tx) = pending.lock().await.remove(&id) {
            let _ = tx.send(result);
        }
        return;
    }
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        let _ = notify_tx
            .send(CodexNotification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            })
            .await;
    }
}

async fn reply_unsupported_request(id: u64, writer: &Arc<Mutex<RpcWriter>>) {
    let frame = serde_json::json!({
        "id": id,
        "error": {
            "code": -32601,
            "message": "Archon Codex app-server adapter does not expose provider-side tools",
        }
    });
    let Ok(line) = serde_json::to_string(&frame) else {
        return;
    };
    let mut writer = writer.lock().await;
    match &mut *writer {
        RpcWriter::Stdio(stdin) => {
            let _ = stdin.write_all(line.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
        RpcWriter::WebSocket(ws) => {
            let _ = ws.send(Message::Text(line.into())).await;
        }
    }
}

async fn reject_all(pending: PendingMap, message: &str) {
    let mut pending = pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(LlmError::Http(message.to_string())));
    }
}

async fn drain_stderr(mut stderr: tokio::process::ChildStderr) {
    let mut buf = [0_u8; 1024];
    while matches!(stderr.read(&mut buf).await, Ok(n) if n > 0) {}
}

fn websocket_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        url.to_string()
    }
}

fn normalize_transport(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_preserves_ws_and_wss() {
        assert_eq!(
            websocket_url("ws://127.0.0.1:11434/codex"),
            "ws://127.0.0.1:11434/codex"
        );
        assert_eq!(
            websocket_url("wss://codex.example.invalid/app-server"),
            "wss://codex.example.invalid/app-server"
        );
    }

    #[test]
    fn websocket_url_converts_http_compatibility_schemes() {
        assert_eq!(
            websocket_url("http://127.0.0.1:11434/codex"),
            "ws://127.0.0.1:11434/codex"
        );
        assert_eq!(
            websocket_url("https://codex.example.invalid/app-server"),
            "wss://codex.example.invalid/app-server"
        );
    }
}
