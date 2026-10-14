use std::collections::BTreeMap;
use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[path = "workflow_decomposition_offline_server_http.rs"]
mod http;
use http::next_request;

#[derive(Clone, Debug, Default)]
pub struct Counts(pub Arc<Mutex<BTreeMap<String, usize>>>);

impl Counts {
    pub fn snapshot(&self) -> BTreeMap<String, usize> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub method: String,
    pub path: String,
    pub auth_present: bool,
    pub known_path: bool,
    pub role: String,
    /// The call the request belongs to (`variants::request_key`).
    pub key: String,
    pub prompt: String,
    pub answer: String,
}

#[derive(Clone, Debug, Default)]
pub struct Requests(pub Arc<Mutex<Vec<RequestRecord>>>);

impl Requests {
    pub fn snapshot(&self) -> Vec<RequestRecord> {
        self.0.lock().unwrap().clone()
    }

    /// How many requests each call made (`RequestRecord::key`).
    pub fn per_call(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for request in self.0.lock().unwrap().iter() {
            *counts.entry(request.key.clone()).or_default() += 1;
        }
        counts
    }
}

/// How a fake provider answers: which reply variant, and which request (the
/// `nth` of `role`, from 1) it holds until `release`.
#[derive(Clone, Debug)]
pub struct Plan {
    pub variant: Variant,
    pub block: Option<(String, usize)>,
}

pub struct FakeProvider {
    pub base_url: String,
    pub counts: Counts,
    pub requests: Requests,
    variant: Arc<Mutex<Variant>>,
    blocked: Option<(String, usize)>,
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
    entered: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeProvider {
    pub fn start() -> Self {
        Self::start_with(Plan {
            variant: Variant::Golden,
            block: None,
        })
    }

    pub fn start_with(plan: Plan) -> Self {
        let Plan { variant, block } = plan;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let counts = Counts::default();
        let request_counts = counts.clone();
        let requests = Requests::default();
        let request_log = requests.clone();
        let variant = Arc::new(Mutex::new(variant));
        let thread_variant = variant.clone();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let thread_release = release.clone();
        let thread_entered = entered.clone();
        let thread_blocked = block.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            let mut connections: Vec<(thread::JoinHandle<()>, std::net::TcpStream)> = Vec::new();
            while !thread_stop.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // An accepted socket inherits the listener's
                        // non-blocking mode on BSD/macOS; a reader would then
                        // see WouldBlock between pooled requests and drop the
                        // connection the client is about to reuse.
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let shutdown_stream = match stream.try_clone() {
                            Ok(stream) => stream,
                            Err(_) => continue,
                        };
                        let counts = request_counts.clone();
                        let requests = request_log.clone();
                        let blocked = thread_blocked.clone();
                        let variant = thread_variant.clone();
                        let release = thread_release.clone();
                        let entered = thread_entered.clone();
                        let stop = thread_stop.clone();
                        connections.push((
                            thread::spawn(move || {
                                serve_connection(
                                    &mut stream,
                                    &counts,
                                    &requests,
                                    &blocked,
                                    variant,
                                    &release,
                                    &entered,
                                    &stop,
                                );
                            }),
                            shutdown_stream,
                        ));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
                let mut active = Vec::new();
                for (connection, shutdown_stream) in connections {
                    if connection.is_finished() {
                        let _ = connection.join();
                    } else {
                        active.push((connection, shutdown_stream));
                    }
                }
                connections = active;
            }
            // Stop is the only server-initiated reason to close a keep-alive
            // socket. Shutting down each tracked socket unblocks its reader.
            for (_, stream) in &connections {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            for (connection, _) in connections {
                let _ = connection.join();
            }
        });
        Self {
            base_url: format!("http://{address}/v1/messages"),
            counts,
            requests,
            variant,
            blocked: block,
            release,
            entered,
            stop,
            thread: Some(thread),
        }
    }

    pub fn wait_until_blocked(&self, wait_interval: Duration) -> bool {
        assert!(self.blocked.is_some(), "provider has no blocked request");
        let mut entered = self.entered.0.lock().unwrap();
        while !*entered {
            let (next, timeout) = self.entered.1.wait_timeout(entered, wait_interval).unwrap();
            entered = next;
            if timeout.timed_out() && !*entered {
                return false;
            }
        }
        true
    }

    pub fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }

    /// Change the reply plan and restart per-role ordinals for a later CLI
    /// launch against the same local provider.
    pub fn set_variant(&self, variant: Variant) {
        *self.variant.lock().unwrap() = variant;
        self.counts.0.lock().unwrap().clear();
        self.requests.0.lock().unwrap().clear();
    }
}

fn serve_connection(
    stream: &mut std::net::TcpStream,
    counts: &Counts,
    requests: &Requests,
    blocked: &Option<(String, usize)>,
    variant: Arc<Mutex<Variant>>,
    release: &Arc<(Mutex<bool>, std::sync::Condvar)>,
    entered: &Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: &std::sync::atomic::AtomicBool,
) {
    let mut pending = Vec::new();
    loop {
        let request = match next_request(stream, &mut pending) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(ref error) if error.kind() == std::io::ErrorKind::TimedOut => {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                continue;
            }
            Err(_) => return,
        };
        let keep_alive = request.keep_alive();
        let auth = request
            .header("x-api-key")
            .map(|value| value == "offline-test-key");
        let bearer = request
            .header("authorization")
            .map(|value| value == "Bearer offline-test-key");
        let auth_present = auth.is_some() || bearer.is_some();
        let auth_valid = auth.unwrap_or(false) || bearer.unwrap_or(false);
        let known_path = request.method == "POST" && request.path == "/v1/messages";
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let prompt = body.to_string();
        let variant = *variant.lock().unwrap();
        let (role, answer) = variants::answer(&prompt, variant);
        let nth = {
            let mut counts = counts.0.lock().unwrap();
            let count = counts.entry(role.to_string()).or_default();
            *count += 1;
            *count
        };
        let answer = variants::refute_first_judge_answer(role, &prompt, answer, variant, nth);
        requests.0.lock().unwrap().push(RequestRecord {
            method: request.method,
            path: request.path,
            auth_present,
            known_path,
            role: role.to_string(),
            key: variants::request_key(role, &prompt),
            prompt,
            answer: answer.clone(),
        });
        if blocked
            .as_ref()
            .is_some_and(|(blocked_role, blocked_nth)| blocked_role == role && *blocked_nth == nth)
        {
            *entered.0.lock().unwrap() = true;
            entered.1.notify_all();
            let mut is_released = release.0.lock().unwrap();
            while !*is_released {
                is_released = release.1.wait(is_released).unwrap();
            }
        }

        let escaped = serde_json::to_string(&answer).unwrap();
        let (status, content_type, response) = if !known_path {
            (
                "404 Not Found",
                "application/json",
                "{\"error\":\"unknown path\"}".to_string(),
            )
        } else if !auth_valid {
            (
                "401 Unauthorized",
                "application/json",
                "{\"error\":\"invalid test key\"}".to_string(),
            )
        } else if role == "unknown" {
            (
                "500 Internal Server Error",
                "application/json",
                answer.clone(),
            )
        } else if body["stream"] == false {
            (
                "200 OK",
                "application/json",
                serde_json::json!({
                    "id": "msg-local", "type": "message", "role": "assistant",
                    "content": [{"type": "text", "text": answer}], "model": "offline",
                    "stop_reason": "end_turn", "stop_sequence": null,
                    "usage": {"input_tokens": 1, "output_tokens": 1}
                })
                .to_string(),
            )
        } else {
            (
                "200 OK",
                "text/event-stream",
                format!(
                    "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-local\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"offline\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\nevent: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{escaped}}}}}\n\nevent: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":1}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
                ),
            )
        };
        let connection = if keep_alive { "keep-alive" } else { "close" };
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n",
            response.len()
        );
        if stream.write_all(header.as_bytes()).is_err()
            || stream.write_all(response.as_bytes()).is_err()
            || stream.flush().is_err()
            || !keep_alive
        {
            return;
        }
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        eprintln!(
            "fake provider received {} requests",
            self.requests.snapshot().len()
        );
        self.release();
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(
            self.base_url
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap(),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[path = "workflow_decomposition_offline_server_script.rs"]
mod script;
#[path = "workflow_decomposition_offline_server_variants.rs"]
pub mod variants;
pub use variants::Variant;

#[cfg(test)]
mod tests {
    use super::{FakeProvider, next_request};
    use std::io::Cursor;

    #[test]
    fn reads_exact_serialized_request_bodies_and_preserves_keep_alive_pipeline() {
        let first_body = serde_json::json!({
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Adversarially judge AC-X-001"}]
        })
        .to_string();
        let second_body = serde_json::json!({"stream": false, "messages": []}).to_string();
        let request = |body: &str| {
            format!(
                "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
        };
        let wire = format!("{}{}", request(&first_body), request(&second_body));
        let mut reader = Cursor::new(wire.into_bytes());
        let mut pending = Vec::new();

        let first = next_request(&mut reader, &mut pending).unwrap().unwrap();
        assert_eq!(first.method, "POST");
        assert_eq!(first.path, "/v1/messages");
        assert!(first.keep_alive());
        assert_eq!(first.body, first_body.as_bytes());

        let second = next_request(&mut reader, &mut pending).unwrap().unwrap();
        assert_eq!(second.body, second_body.as_bytes());
        assert!(second.keep_alive());
        assert!(next_request(&mut reader, &mut pending).unwrap().is_none());
    }

    #[test]
    fn pooled_client_sends_fifty_sequential_requests_without_errors() {
        let provider = FakeProvider::start();
        let client = reqwest::blocking::Client::builder()
            .pool_idle_timeout(None)
            .build()
            .unwrap();

        for index in 0..50 {
            let response = client
                .post(&provider.base_url)
                .header("x-api-key", "offline-test-key")
                .json(&serde_json::json!({
                    "stream": false,
                    "messages": [{"role": "user", "content": format!("request {index}")}]
                }))
                .send()
                .unwrap_or_else(|error| panic!("request {index} failed: {error}"));
            // Unknown prompts intentionally return HTTP 500, which still
            // proves the keep-alive exchange completed and its body was read.
            assert_eq!(
                response.status(),
                reqwest::StatusCode::INTERNAL_SERVER_ERROR
            );
            let _ = response.bytes().unwrap();
        }

        assert_eq!(provider.requests.snapshot().len(), 50);
        assert_eq!(provider.counts.snapshot().values().sum::<usize>(), 50);
        drop(client);
        drop(provider);
    }
}
