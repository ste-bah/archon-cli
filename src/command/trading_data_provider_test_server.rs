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
    /// Wait for the server thread and fail the test with whatever it reported.
    ///
    /// Nested `unwrap()`s here would print `Err("...")` from a `Result<Result<>>`
    /// and lose which layer failed; the server's own error string is the only
    /// description of what the client got wrong.
    pub(super) fn join(mut self) {
        let handle = self.handle.take().expect("mock OpenBB server joined once");
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("mock OpenBB server: {error}"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
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

/// How long the mock server waits for the request once a client has connected.
///
/// Sibling of `REQUEST_DEADLINE`, which bounds the wait for the connection
/// itself, and generous for the same reason: a client that connects and then
/// never finishes its request must not hang the suite, but the bound is a hang
/// guard and not a claim about how fast a loopback write should be.
const READ_DEADLINE: Duration = Duration::from_secs(60);

/// Largest request head this mock will buffer before refusing.
///
/// The clients under test send GETs a few hundred bytes long. A head past this
/// is a client defect; refusing tells the test that, where reading on would
/// block until `READ_DEADLINE` and report a timeout instead.
const MAX_REQUEST_HEAD: usize = 64 * 1024;

fn serve_response(
    mut stream: TcpStream,
    body: &str,
    content_type: &str,
    expected_parts: &[&str],
) -> Result<(), String> {
    let request = read_request_head(&mut stream)?;
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
        .map_err(|error| format!("mock OpenBB response write failed: {error}"))
}

/// Read the request head — everything up to the blank line that terminates it.
///
/// One 4096-byte `read()` was wrong on two counts, and both let the
/// `expected_parts` check above assert against a fragment of a request.
///
/// First, the accepted socket inherited the listener's non-blocking mode on
/// Windows, so a `read()` issued before the client's bytes landed failed with
/// WSAEWOULDBLOCK instead of waiting. That is fixed where the socket is
/// accepted — see [`accept_before_deadline`], which explains it in full — so by
/// the time it arrives here it blocks, and the read timeout below is what keeps
/// that wait bounded.
///
/// Second, TCP may split a request across segments, so even a blocking read can
/// return a prefix. Looping to the end of the head is the only way to know the
/// whole request was seen. That half is this function's own job.
fn read_request_head(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(READ_DEADLINE))
        .map_err(|error| format!("mock OpenBB could not bound the request read: {error}"))?;
    let mut head = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&head).into_owned());
        }
        if head.len() > MAX_REQUEST_HEAD {
            return Err(format!(
                "mock OpenBB request head passed {MAX_REQUEST_HEAD} bytes with no blank line"
            ));
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err("mock OpenBB client closed mid-request".into()),
            Ok(read) => head.extend_from_slice(&chunk[..read]),
            Err(error) => return Err(format!("mock OpenBB request read failed: {error}")),
        }
    }
}
