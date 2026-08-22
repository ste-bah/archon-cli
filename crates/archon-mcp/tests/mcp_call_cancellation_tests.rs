//! Cancellation safety for the MCP tool call budget (#200 Phase 1).
//!
//! Opting `McpTool` into the dispatcher's per-call budget means its `execute`
//! future is dropped at the deadline, mid-await, with a `tools/call` already on
//! the wire. The resource at stake is not local — it is the *server's* worker,
//! which will keep computing an answer nobody will read unless it is told to
//! stop. These tests drive a real `rmcp` client against an in-process server
//! that deliberately stalls, and require the cancellation to arrive.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archon_mcp::client::McpClient;
use archon_mcp::tool_bridge::McpTool;
use archon_mcp::types::{McpToolDef, ServerConfig};
use archon_tools::execution_deadline::ExecutionDeadline;
use archon_tools::tool::{Tool, ToolContext};
use rmcp::service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

// ---------------------------------------------------------------------------
// In-process transport
// ---------------------------------------------------------------------------

/// Carries client→server traffic as raw JSON so the fake server can inspect it
/// by method name without depending on rmcp's request enums.
struct ChannelTransport {
    to_server: UnboundedSender<Value>,
    from_server: UnboundedReceiver<RxJsonRpcMessage<RoleClient>>,
}

impl Transport<RoleClient> for ChannelTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let to_server = self.to_server.clone();
        async move {
            let encoded = serde_json::to_value(&item).map_err(std::io::Error::other)?;
            to_server
                .send(encoded)
                .map_err(|_| std::io::Error::other("fake MCP server is gone"))
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.from_server.recv().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fake MCP server
// ---------------------------------------------------------------------------

/// Everything the client has sent, in order.
type Traffic = Arc<Mutex<Vec<Value>>>;

/// Answers `initialize`; answers `tools/call` only if `answer_calls` is set.
async fn run_fake_server(
    mut inbox: UnboundedReceiver<Value>,
    outbox: UnboundedSender<RxJsonRpcMessage<RoleClient>>,
    traffic: Traffic,
    answer_calls: bool,
) {
    while let Some(message) = inbox.recv().await {
        traffic.lock().expect("traffic lock").push(message.clone());

        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue; // a notification; nothing to answer
        };

        let result = match method {
            "initialize" => json!({
                // Echo the client's version so this never drifts with rmcp.
                "protocolVersion": message
                    .pointer("/params/protocolVersion")
                    .cloned()
                    .expect("initialize carries a protocol version"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "stalling-test-server", "version": "0.0.0" }
            }),
            "tools/call" if answer_calls => json!({
                "content": [{ "type": "text", "text": "server answered" }],
                "isError": false
            }),
            // Otherwise: deliberately no response, ever.
            _ => continue,
        };

        let response: RxJsonRpcMessage<RoleClient> =
            serde_json::from_value(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
                .expect("fake server response is a valid JSON-RPC message");
        if outbox.send(response).is_err() {
            return;
        }
    }
}

fn server_config() -> ServerConfig {
    ServerConfig {
        name: "stall".into(),
        command: "unused".into(),
        args: vec![],
        env: std::collections::HashMap::new(),
        disabled: false,
        transport: "stdio".into(),
        url: None,
        headers: None,
        allow_insecure_ws: false,
        tool_policy: Default::default(),
    }
}

fn tool_def() -> McpToolDef {
    McpToolDef {
        name: "stalling_tool".into(),
        description: Some("never answers".into()),
        input_schema: json!({ "type": "object" }),
        annotations: None,
        meta: None,
        server_name: "stall".into(),
    }
}

/// Connect a client to a fake server and hand back the tool plus the traffic log.
async fn connect(answer_calls: bool) -> (McpTool, Traffic, Arc<McpClient>) {
    let (client_tx, server_rx) = unbounded_channel();
    let (server_tx, client_rx) = unbounded_channel();
    let traffic: Traffic = Arc::new(Mutex::new(Vec::new()));

    tokio::spawn(run_fake_server(
        server_rx,
        server_tx,
        Arc::clone(&traffic),
        answer_calls,
    ));

    let transport = ChannelTransport {
        to_server: client_tx,
        from_server: client_rx,
    };
    let client = Arc::new(
        McpClient::initialize(&server_config(), transport)
            .await
            .expect("handshake with the fake server"),
    );
    let tool = McpTool::new("stall", tool_def(), Arc::clone(&client));
    (tool, traffic, client)
}

fn find(traffic: &Traffic, method: &str) -> Option<Value> {
    traffic
        .lock()
        .expect("traffic lock")
        .iter()
        .find(|message| message.get("method").and_then(Value::as_str) == Some(method))
        .cloned()
}

/// Poll the traffic log until `method` shows up or `budget` runs out.
async fn wait_for(traffic: &Traffic, method: &str, budget: Duration) -> Option<Value> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if let Some(message) = find(traffic, method) {
            return Some(message);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_tools_declare_a_call_budget() {
    let (tool, _traffic, _client) = connect(false).await;
    let budget = tool.timeout().expect("MCP tools declare a budget");
    assert!(
        budget > Duration::from_secs(120),
        "the budget is a backstop above McpClient's own 120s response wait, \
         so the specific error still wins on an ordinary slow server"
    );
}

#[tokio::test]
async fn dropping_an_mcp_call_at_the_deadline_cancels_it_on_the_server() {
    let (tool, traffic, _client) = connect(false).await;

    let outcome = ExecutionDeadline::new(Duration::from_millis(300))
        .wait(tool.execute(json!({ "value": 1 }), &ToolContext::default()))
        .await;

    assert!(
        outcome.is_none(),
        "the stalled MCP call must not have produced a result"
    );

    let call = find(&traffic, "tools/call").expect("the call reached the server");
    let cancelled = wait_for(&traffic, "notifications/cancelled", Duration::from_secs(10))
        .await
        .expect(
            "dropping the call must tell the server to stop; without that the server keeps \
         working on an answer nobody will read",
        );

    assert_eq!(
        cancelled.pointer("/params/requestId"),
        call.get("id"),
        "the cancellation must name the abandoned request"
    );
}

#[tokio::test]
async fn a_call_that_completes_is_not_cancelled() {
    let (tool, traffic, _client) = connect(true).await;

    let result = tool
        .execute(json!({ "value": 1 }), &ToolContext::default())
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(result.content, "server answered");

    // Give the guard the same window the cancellation test allows, then require
    // silence: a guard that fired on the success path would cancel calls the
    // server had already finished.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        find(&traffic, "notifications/cancelled").is_none(),
        "a completed call must not be cancelled"
    );
}
