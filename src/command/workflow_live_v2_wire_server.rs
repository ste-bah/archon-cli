//! Local HTTP capture for the workflow wire regression.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn read_raw_request(mut socket: tokio::net::TcpStream) -> (tokio::net::TcpStream, Vec<u8>) {
    let mut request = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.expect("read request");
        assert!(read > 0, "connection closed before headers completed");
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let content_length = content_length(&request[..header_end]);
    while request.len() - header_end < content_length {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.expect("read body");
        assert!(read > 0, "connection closed before body completed");
        request.extend_from_slice(&buffer[..read]);
    }
    (
        socket,
        request[header_end..header_end + content_length].to_vec(),
    )
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse().ok())
        .expect("content length")
}

pub(super) async fn serve_two_anthropic_requests(
    listener: TcpListener,
    captured: tokio::sync::oneshot::Sender<Vec<Vec<u8>>>,
) {
    let mut bodies = Vec::new();
    for index in 0..2 {
        // The child-process deadline bounds setup and missing connections.
        // The read budget belongs only to an actual connected request.
        let (socket, _) = listener.accept().await.expect("accept request");
        let (mut socket, body) =
            tokio::time::timeout(std::time::Duration::from_secs(10), read_raw_request(socket))
                .await
                .expect("request capture timed out");
        bodies.push(body);
        write_anthropic_response(&mut socket, index).await;
    }
    captured.send(bodies).expect("send captured bodies");
}

async fn write_anthropic_response(socket: &mut tokio::net::TcpStream, index: usize) {
    let (input_tokens, cache_creation_tokens, cache_read_tokens, output_tokens) = match index {
        0 => (11, 3, 0, 7),
        1 => (13, 0, 11, 9),
        _ => unreachable!("wire fixture serves exactly two requests"),
    };
    let message_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": format!("msg-{index}"),
            "model": "claude-sonnet-4-6",
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": 0,
                "cache_creation_input_tokens": cache_creation_tokens,
                "cache_read_input_tokens": cache_read_tokens,
            }
        }
    });
    let message_delta = serde_json::json!({
        "type": "message_delta",
        "delta": {"stop_reason": "end_turn"},
        "usage": {"output_tokens": output_tokens},
    });
    let response = format!(
        "event: message_start\ndata: {message_start}\n\n\
         event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
         event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"recorded\"}}}}\n\n\
         event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
         event: message_delta\ndata: {message_delta}\n\n\
         event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
    );
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
        response.len()
    );
    socket
        .write_all(headers.as_bytes())
        .await
        .expect("write headers");
    socket
        .write_all(response.as_bytes())
        .await
        .expect("write body");
}

#[tokio::test(start_paused = true)]
async fn setup_time_is_not_a_request_read_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_two_anthropic_requests(listener, tx));
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    tokio::task::yield_now().await;
    let expired = server.is_finished();
    server.abort();
    let _ = server.await;
    assert!(
        !expired,
        "fixture setup must not consume the request read budget"
    );
}
