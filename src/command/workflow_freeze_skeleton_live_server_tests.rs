use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Server {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<usize>>,
}
impl Server {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut count = 0;
            while !done.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = Vec::new();
                let end = loop {
                    let mut b = [0; 8192];
                    let n = socket.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..end]);
                let len: usize = header
                    .lines()
                    .find_map(|l| {
                        l.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < end + len {
                    let mut b = [0; 8192];
                    let n = socket.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                }
                let request: serde_json::Value =
                    serde_json::from_slice(&bytes[end..end + len]).unwrap();
                let prompt = request.to_string();
                let name = if prompt.contains("Pick one function or block") {
                    "breaker_attempt1_gut1"
                } else if prompt.contains("Stated phrase:") {
                    "relevant"
                } else if prompt.contains("Compare observable values") {
                    "equivalent"
                } else if prompt.contains("Return 2 to 5 behaviour mutants") {
                    "breaker_attempt1"
                } else {
                    panic!("unknown proof prompt: {prompt}")
                };
                eprintln!(
                    "skeleton corpus request: {name}, model={}",
                    request["model"]
                );
                let answer = std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/breaker_reply_corpus")
                        .join(format!("{name}.txt")),
                )
                .unwrap();
                let message = serde_json::json!({"id":"msg-corpus","type":"message","role":"assistant","content":[{"type":"text","text":answer}],"model":"offline","stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}});
                let (kind, response) = if request["stream"] == false {
                    ("application/json", message.to_string())
                } else {
                    let start = serde_json::json!({"type":"message_start","message":{"id":"msg-corpus","type":"message","role":"assistant","content":[],"model":"offline","usage":{"input_tokens":1,"output_tokens":0}}});
                    let delta = serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":answer}});
                    (
                        "text/event-stream",
                        format!(
                            "event: message_start\ndata: {start}\n\nevent: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\nevent: content_block_delta\ndata: {delta}\n\nevent: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":1}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
                        ),
                    )
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
                count += 1;
            }
            count
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
    pub fn finish(mut self) -> usize {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
