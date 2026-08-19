//! End-to-end tests over a real pair of streams (#189 Phase 11).
//!
//! These drive the loop the way an editor does — lines in, lines out — so the
//! three acceptance criteria are asserted against what actually crosses the
//! connection rather than against the functions behind it.

use super::*;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::protocol::{ContentBlock, SessionUpdate, StopReason};

/// A stand-in agent. Records what it was asked, answers what it was told to.
#[derive(Default)]
struct StubAgent {
    cancelled: AtomicBool,
    /// Set to have `prompt` ask for permission before answering.
    ask_permission: bool,
    prompts: Mutex<Vec<String>>,
    permission_granted: Mutex<Option<bool>>,
}

#[async_trait::async_trait]
impl AcpAgent for StubAgent {
    async fn new_session(&self, cwd: &str) -> anyhow::Result<String> {
        Ok(format!("sess-for-{cwd}"))
    }

    async fn prompt(&self, session_id: &str, text: &str, peer: Arc<Peer>) -> StopReason {
        self.prompts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(text.to_string());

        if self.ask_permission {
            let granted = peer
                .request_permission(session_id, "call_001", "Run a command")
                .await;
            *self
                .permission_granted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(granted);
        }

        peer.update(
            session_id,
            SessionUpdate::AgentMessageChunk {
                content: ContentBlock::text("answering"),
            },
        );

        if self.cancelled.load(Ordering::SeqCst) {
            return StopReason::Cancelled;
        }
        StopReason::EndTurn
    }

    fn cancel(&self, _session_id: &str) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// Feed `lines` to the loop and collect everything it writes back.
///
/// A cursor over the request lines and a shared sink for the replies, rather
/// than a socket pair: the loop ends when its input ends, so a finite input is
/// what makes these tests terminate without a timeout.
async fn run(agent: Arc<StubAgent>, lines: &[&str]) -> Vec<serde_json::Value> {
    let input = std::io::Cursor::new((lines.join("\n") + "\n").into_bytes());
    let output = SharedSink::default();
    let sink = output.clone();

    serve(input, sink, agent as Arc<dyn AcpAgent>)
        .await
        .expect("the loop runs to end of input");

    output.lines()
}

/// A writer that keeps what was written, so a test can read the replies.
#[derive(Clone, Default)]
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl SharedSink {
    fn lines(&self) -> Vec<serde_json::Value> {
        let bytes = self.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("every written line is JSON"))
            .collect()
    }
}

impl tokio::io::AsyncWrite for SharedSink {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// A live connection, for the tests where ordering is the point.
///
/// The cursor harness above cannot express "answer the question once it is
/// asked": it queues every client line up front, so a reply can arrive before
/// the request it answers. A real editor never does that, and a test that did
/// would be racing its own fixture.
struct Conversation {
    to_agent: tokio::io::DuplexStream,
    from_agent: tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    served: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Conversation {
    fn open(agent: Arc<StubAgent>) -> Self {
        let (to_agent, agent_input) = tokio::io::duplex(64 * 1024);
        let (agent_output, from_agent) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(serve(agent_input, agent_output, agent as Arc<dyn AcpAgent>));
        Self {
            to_agent,
            from_agent: BufReader::new(from_agent).lines(),
            served,
        }
    }

    async fn send(&mut self, line: &str) {
        use tokio::io::AsyncWriteExt;
        self.to_agent
            .write_all(line.as_bytes())
            .await
            .expect("write");
        self.to_agent.write_all(b"\n").await.expect("write");
        self.to_agent.flush().await.expect("flush");
    }

    /// Read until a message whose `method` matches.
    async fn wait_for(&mut self, method: &str) -> serde_json::Value {
        let deadline = std::time::Duration::from_secs(10);
        tokio::time::timeout(deadline, async {
            while let Ok(Some(line)) = self.from_agent.next_line().await {
                let value: serde_json::Value =
                    serde_json::from_str(&line).expect("every written line is JSON");
                if value["method"] == method {
                    return value;
                }
            }
            panic!("the connection ended before {method} arrived");
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {method}"))
    }

    /// Close the client end and let the loop finish.
    async fn close(self) {
        drop(self.to_agent);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), self.served)
            .await
            .expect("the loop terminates when its input ends");
    }
}

fn reply_to(lines: &[serde_json::Value], id: i64) -> Option<&serde_json::Value> {
    lines.iter().find(|line| line["id"] == id)
}

fn notifications<'a>(lines: &'a [serde_json::Value], method: &str) -> Vec<&'a serde_json::Value> {
    lines
        .iter()
        .filter(|line| line["method"] == method)
        .collect()
}

#[tokio::test]
async fn initialize_reports_the_protocol_version_and_names_the_agent() {
    let lines = run(
        Arc::new(StubAgent::default()),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#],
    )
    .await;

    let reply = reply_to(&lines, 1).expect("initialize is answered");
    assert_eq!(reply["result"]["protocolVersion"], 1);
    assert_eq!(reply["result"]["agentInfo"]["name"], "archon");
}

/// The first acceptance criterion: a client connects, sends a prompt, and
/// receives streamed output.
#[tokio::test]
async fn a_client_prompt_is_answered_with_streamed_output_and_a_stop_reason() {
    let agent = Arc::new(StubAgent::default());
    let lines = run(
        Arc::clone(&agent),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp/p","mcpServers":[]}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"sess-for-/tmp/p","prompt":[{"type":"text","text":"fix the build"}]}}"#,
        ],
    )
    .await;

    assert_eq!(
        reply_to(&lines, 1).expect("session/new is answered")["result"]["sessionId"],
        "sess-for-/tmp/p"
    );

    let streamed = notifications(&lines, "session/update");
    assert!(!streamed.is_empty(), "no output was streamed: {lines:?}");
    assert_eq!(
        streamed[0]["params"]["update"]["sessionUpdate"],
        "agent_message_chunk"
    );
    assert_eq!(
        streamed[0]["params"]["update"]["content"]["text"],
        "answering"
    );

    assert_eq!(
        reply_to(&lines, 2).expect("the prompt is answered")["result"]["stopReason"],
        "end_turn"
    );
    assert_eq!(
        agent
            .prompts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_slice(),
        ["fix the build"]
    );
}

/// Ask, answer with `outcome`, and report what the agent was told.
async fn answer_permission_with(outcome: serde_json::Value) -> Option<bool> {
    let agent = Arc::new(StubAgent {
        ask_permission: true,
        ..StubAgent::default()
    });
    let mut conversation = Conversation::open(Arc::clone(&agent));

    conversation
        .send(
            r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"run it"}]}}"#,
        )
        .await;

    let asked = conversation.wait_for("session/request_permission").await;
    let reply = serde_json::json!({
        "jsonrpc": "2.0",
        "id": asked["id"],
        "result": { "outcome": outcome },
    });
    conversation.send(&reply.to_string()).await;
    conversation.close().await;

    *agent
        .permission_granted
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// The second acceptance criterion: a tool permission request reaches the
/// client and its answer is honoured.
#[tokio::test]
async fn a_permission_request_reaches_the_client_and_its_answer_is_honoured() {
    let agent = Arc::new(StubAgent {
        ask_permission: true,
        ..StubAgent::default()
    });
    let mut conversation = Conversation::open(Arc::clone(&agent));
    conversation
        .send(
            r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"run it"}]}}"#,
        )
        .await;

    let asked = conversation.wait_for("session/request_permission").await;

    assert_eq!(asked["params"]["toolCall"]["toolCallId"], "call_001");
    assert_eq!(asked["params"]["sessionId"], "s1");
    let options = asked["params"]["options"]
        .as_array()
        .expect("options are offered");
    assert!(
        options.iter().any(|option| option["kind"] == "allow_once"),
        "{options:?}"
    );

    conversation
        .send(
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": asked["id"],
                "result": { "outcome": { "outcome": "selected", "optionId": "allow-once" } },
            })
            .to_string(),
        )
        .await;
    conversation.close().await;

    assert_eq!(
        *agent
            .permission_granted
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
        Some(true),
        "the client allowed it and the agent was told otherwise"
    );
}

/// A rejection must be honoured just as exactly, and this is the direction that
/// matters: a mis-parsed answer that defaulted to allow would run what the user
/// refused. A dismissed prompt is a refusal too — "I could not tell" has to
/// mean no.
#[tokio::test]
async fn every_answer_that_is_not_an_allow_is_a_refusal() {
    for outcome in [
        serde_json::json!({ "outcome": "selected", "optionId": "reject-once" }),
        serde_json::json!({ "outcome": "cancelled" }),
        serde_json::json!({ "outcome": "selected", "optionId": "never-offered" }),
    ] {
        assert_eq!(
            answer_permission_with(outcome.clone()).await,
            Some(false),
            "this answer should not have granted permission: {outcome}"
        );
    }
}

#[tokio::test]
async fn an_allowed_permission_is_granted() {
    assert_eq!(
        answer_permission_with(
            serde_json::json!({ "outcome": "selected", "optionId": "allow-once" })
        )
        .await,
        Some(true)
    );
}

/// The third acceptance criterion: cancellation from the client stops the turn.
#[tokio::test]
async fn cancellation_from_the_client_reaches_the_agent() {
    let agent = Arc::new(StubAgent::default());
    run(
        Arc::clone(&agent),
        &[r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s1"}}"#],
    )
    .await;

    assert!(
        agent.cancelled.load(Ordering::SeqCst),
        "the notification never reached the agent"
    );
}

#[tokio::test]
async fn an_unknown_method_is_refused_by_name_rather_than_ignored() {
    let lines = run(
        Arc::new(StubAgent::default()),
        &[r#"{"jsonrpc":"2.0","id":9,"method":"session/load","params":{}}"#],
    )
    .await;

    let reply = reply_to(&lines, 9).expect("even an unknown method is answered");
    assert_eq!(reply["error"]["code"], crate::jsonrpc::METHOD_NOT_FOUND);
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("session/load"),
        "{reply}"
    );
}

/// One malformed line from an editor must not take the session with it.
#[tokio::test]
async fn a_malformed_line_is_skipped_and_the_next_one_is_served() {
    let lines = run(
        Arc::new(StubAgent::default()),
        &[
            "{ this is not json",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ],
    )
    .await;

    assert!(
        reply_to(&lines, 1).is_some(),
        "the connection died on a bad line: {lines:?}"
    );
}

#[tokio::test]
async fn malformed_parameters_are_an_invalid_params_error() {
    let lines = run(
        Arc::new(StubAgent::default()),
        &[r#"{"jsonrpc":"2.0","id":5,"method":"session/prompt","params":{"prompt":"not an array"}}"#],
    )
    .await;

    let reply = reply_to(&lines, 5).expect("answered");
    assert_eq!(reply["error"]["code"], crate::jsonrpc::INVALID_PARAMS);
}
