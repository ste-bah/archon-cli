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

fn accept_before_deadline(listener: &TcpListener, stop: &AtomicBool) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    return Err("mock OpenBB server stopped before a request".into());
                }
                if Instant::now() >= deadline {
                    return Err("mock OpenBB server received no request within 5s".into());
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
