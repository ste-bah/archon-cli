use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub(super) struct MockOpenBbServer {
    pub(super) base_url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl MockOpenBbServer {
    pub(super) fn join(mut self) {
        self.handle.take().unwrap().join().unwrap().unwrap();
    }
}

impl Drop for MockOpenBbServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) fn openbb_server(
    body: serde_json::Value,
    expected_parts: &[&'static str],
) -> MockOpenBbServer {
    http_server(body.to_string(), "application/json", expected_parts)
}

pub(super) fn raw_http_server(
    body: &'static str,
    content_type: &'static str,
    expected_parts: &[&'static str],
) -> MockOpenBbServer {
    http_server(body.to_string(), content_type, expected_parts)
}

fn http_server(
    body: String,
    content_type: &'static str,
    expected_parts: &[&'static str],
) -> MockOpenBbServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let expected_parts = expected_parts.to_vec();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let stream = accept_before_deadline(&listener, &worker_stop)?;
        serve_response(stream, &body, content_type, &expected_parts)
    });
    MockOpenBbServer {
        base_url,
        stop,
        handle: Some(handle),
    }
}

/// How long the mock server waits for its one request before giving up.
///
/// This is a hang guard, not an assertion about speed: without it a test whose
/// client never connects blocks the suite forever. So it should be as generous
/// as a bounded wait allows.
///
/// It was 5s, which is a claim about machine load rather than about
/// correctness, and the claim was false. Under `cargo test` the whole module
/// runs concurrently on a machine that is also linking; the client would
/// occasionally not connect inside 5s, the server thread returned "received no
/// request within 5s", and `join().unwrap()` panicked — while holding the env
/// lock, so the poison took several unrelated tests with it. Single-threaded it
/// never reproduced, which is the signature of a load-sensitive deadline rather
/// than a logic error.
const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

fn accept_before_deadline(listener: &TcpListener, stop: &AtomicBool) -> Result<TcpStream, String> {
    let deadline = Instant::now() + REQUEST_DEADLINE;
    loop {
        match listener.accept() {
            // The listener is non-blocking so the deadline above can be
            // enforced. On Windows the accepted socket INHERITS that mode, so
            // `serve_response`'s first `read` returns WSAEWOULDBLOCK (os error
            // 10035) whenever the client's request bytes have not landed yet —
            // a race the reader lost intermittently under full-suite load and
            // won when run alone, which is why it looked like a flaky test
            // rather than the unconditional bug it is. Restoring blocking mode
            // here is what makes the read wait for the request instead of
            // failing closed on its absence; the deadline has already done its
            // job by this point.
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .map_err(|error| format!("mock OpenBB server could not block: {error}"))?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    return Err("mock OpenBB server stopped before a request".into());
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "mock OpenBB server received no request within {}s",
                        REQUEST_DEADLINE.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("mock OpenBB accept failed: {error}")),
        }
    }
}

fn serve_response(
    mut stream: TcpStream,
    body: &str,
    content_type: &str,
    expected_parts: &[&str],
) -> Result<(), String> {
    let mut buffer = [0_u8; 4096];
    let read = stream
        .read(&mut buffer)
        .map_err(|error| error.to_string())?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    if let Some(expected) = expected_parts
        .iter()
        .find(|expected| !request.contains(**expected))
    {
        return Err(format!("request omitted {expected}: {request}"));
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nX-Api-Key: hidden\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}
