use super::*;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::OpenAiProvider;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_request(socket: &mut TcpStream) -> bool {
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        if socket.read(&mut byte).await.unwrap() == 0 {
            return false;
        }
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
    true
}

async fn answer(socket: &mut TcpStream) {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"AC-X-001\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}

struct SocketClient {
    provider: OpenAiProvider,
    attempts: Mutex<Vec<tokio::time::Instant>>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for SocketClient {
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }

    async fn run_agent(&self, _: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        self.attempts
            .lock()
            .unwrap()
            .push(tokio::time::Instant::now());
        self.provider
            .complete(LlmRequest {
                model: "AC-X-001".into(),
                ..Default::default()
            })
            .await
            .map_err(WorkflowError::port)?;
        Ok(WorkflowAgentOutcome::default())
    }
}

#[tokio::test(start_paused = true)]
async fn llm_retry_recovers_on_fresh_sockets_after_sleep_without_spinning() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut warm, _) = listener.accept().await.unwrap();
        assert!(read_request(&mut warm).await);
        answer(&mut warm).await;
        // Model a peer that silently loses the warmed connection during sleep.
        let (reused, mut failed) = tokio::select! {
            request = read_request(&mut warm) => {
                if request {
                    (true, warm)
                } else {
                    (false, listener.accept().await.unwrap().0)
                }
            }
            accepted = listener.accept() => (false, accepted.unwrap().0),
        };
        if !reused {
            assert!(read_request(&mut failed).await);
        }
        drop(failed);
        // Twelve consecutive connections fail without answering. Recovery
        // must stay in the production helper, with an asynchronous backoff.
        for _ in 1..12 {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_request(&mut socket).await);
        }
        let (mut recovered, _) = listener.accept().await.unwrap();
        assert!(read_request(&mut recovered).await);
        answer(&mut recovered).await;
        reused
    });
    let fake = Arc::new(SocketClient {
        provider: OpenAiProvider::new("AC-X-001".into(), Some(address), String::new()),
        attempts: Mutex::new(Vec::new()),
    });
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    let request = super::tests::request(false);
    client.run_agent(request.clone()).await.unwrap();
    fake.attempts.lock().unwrap().clear();
    // This policy opens a fresh connection even with zero monotonic elapsed
    // time, so an arbitrary intervening wall-clock jump cannot retain a pool.
    run_agent_with_transient_retry(&client, request, |_| async { Ok(()) })
        .await
        .unwrap();
    assert!(
        !server.await.unwrap(),
        "attempt after sleep reused the dead socket"
    );
    let attempts = fake.attempts.lock().unwrap();
    assert_eq!(attempts.len(), 13);
    for (index, pair) in attempts.windows(2).enumerate() {
        let expected = Duration::from_millis(500 * (1u64 << index).min(16));
        assert_eq!(pair[1] - pair[0], expected);
    }
    assert_eq!(
        attempts
            .iter()
            .filter(|time| **time - attempts[0] < Duration::from_secs(60))
            .count(),
        11
    );
}
