//! The re-ask through the real provider client: a local HTTP server speaks
//! the Anthropic SSE protocol, the first reply carries one stray key, and the
//! second request must carry that reply and its error as real wire turns.
use super::*;
use crate::runtime::llm::transport_tests::{PING, answer};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serve one scripted SSE reply per POST, in order, recording each body.
async fn serve(replies: Vec<String>) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let recorded = bodies.clone();
    tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let body = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0, "request closed early");
                request.extend_from_slice(&buffer[..n]);
                let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") else {
                    continue;
                };
                let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let len: usize = header
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .unwrap_or("0")
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + len {
                    break request[end + 4..end + 4 + len].to_vec();
                }
            };
            recorded
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&body).unwrap());
            let sse = format!("{PING}{}", answer(&reply));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
                sse.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        }
    });
    (endpoint, bodies)
}

fn client(endpoint: String) -> Arc<dyn WorkflowLlmClient> {
    use archon_llm::{
        anthropic::AnthropicClient,
        auth::AuthProvider,
        identity::{IdentityMode, IdentityProvider},
        provider::LlmProvider,
        providers::AnthropicProvider,
        types::Secret,
    };
    let provider = Arc::new(AnthropicProvider::new(AnthropicClient::with_read_backstop(
        AuthProvider::ApiKey(Secret::new("fixture-key".into())),
        IdentityProvider::new(
            IdentityMode::Clean,
            "fixture".into(),
            "device".into(),
            String::new(),
        ),
        Some(endpoint),
        30,
    ))) as Arc<dyn LlmProvider>;
    let adapter = archon_pipeline::llm_adapter::ProviderLlmAdapter::new(provider);
    crate::command::pipeline_workflow_llm::PipelineWorkflowLlmClient::arc(Arc::new(adapter))
}

#[tokio::test]
async fn a_stray_key_then_a_corrected_reply_is_judged_through_the_real_provider_client() {
    let obligations = [ClaimedObligation {
        id: "REQ-X-001".into(),
        text: "required behavior".into(),
    }];
    let tasks = [ClaimingTask {
        task_id: "TASK-X-001".into(),
        text: "required behavior".into(),
    }];
    let good = serde_json::json!({"verdicts": [{"obligation_id": "REQ-X-001", "necessarily_true": true, "weakest_task_id": "", "reason": "the task obliges it", "quoted_task_text": ""}]});
    let mut stray = good.clone();
    stray["verdicts"][0]["weakest_task_text"] = "".into();
    let (endpoint, bodies) = serve(vec![stray.to_string(), good.to_string()]).await;
    let cache = tempfile::tempdir().unwrap();
    let asked = ask(
        client(endpoint).as_ref(),
        cache.path(),
        "digest",
        &obligations,
        &tasks,
        &SkeletonSummary::absent(),
        &FreezeBudget::unlimited(),
    )
    .await
    .expect("answered");
    let Asked::Answered(verdicts) = asked else {
        panic!("stopped, not answered");
    };
    assert_eq!(verdicts.len(), 1);
    assert!(verdicts[0].necessarily_true);
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    let messages = bodies[1]["messages"].as_array().expect("messages");
    let roles: Vec<_> = messages.iter().map(|m| m["role"].as_str()).collect();
    assert_eq!(roles, [Some("user"), Some("assistant"), Some("user")]);
    let wire = serde_json::to_string(messages).unwrap();
    assert!(
        wire.contains("weakest_task_text") && wire.contains("unknown field"),
        "the rejected reply and its error travel on the wire: {wire}"
    );
    assert!(cache.path().join("rejected/digest-attempt-1.txt").is_file());
}
