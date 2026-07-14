use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Secret;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn client(url: String) -> AnthropicClient {
    AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        ),
        Some(url),
    )
}

async fn accept_request(listener: &TcpListener) -> tokio::net::TcpStream {
    let (mut socket, _) = listener.accept().await.expect("accept request");
    let mut request = Vec::new();
    loop {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.expect("read request");
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return socket;
        }
    }
}

#[tokio::test]
async fn anthropic_stream_emits_fragmented_first_event_before_response_completes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );
    let response_completed = Arc::new(AtomicBool::new(false));
    let completion_flag = response_completed.clone();

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .expect("write headers");
        let event = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n";
        let split = event.len() / 2;
        for chunk in [&event[..split], &event[split..]] {
            socket
                .write_all(format!("{:X}\r\n", chunk.len()).as_bytes())
                .await
                .expect("write chunk length");
            socket.write_all(chunk).await.expect("write chunk");
            socket
                .write_all(b"\r\n")
                .await
                .expect("write chunk terminator");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        completion_flag.store(true, Ordering::SeqCst);
        socket
            .write_all(b"0\r\n\r\n")
            .await
            .expect("finish response");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let first = tokio::time::timeout(Duration::from_millis(100), stream.recv())
        .await
        .expect("first event should arrive before response completion")
        .expect("stream should emit first event");

    assert!(matches!(first, StreamEvent::MessageStart { id, .. } if id == "msg_1"));
    assert!(!response_completed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn anthropic_stream_reports_protocol_error_when_eof_precedes_message_stop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n"
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(body.as_bytes()).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [
            StreamEvent::MessageStart { .. },
            StreamEvent::TextDelta { text, .. },
            StreamEvent::Error { error_type, .. }
        ] if text == "partial" && error_type == "protocol"
    ));
}

#[tokio::test]
async fn anthropic_stream_concatenates_multiline_data_fields_before_parsing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let body = concat!(
            "event: message_start\r\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_multiline\",\r\n",
            "data: \"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\r\n\r\n",
            ": ignored comment\r\n",
            "event: message_stop\r\n",
            "data: {}\r\n\r\n"
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(body.as_bytes()).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [StreamEvent::MessageStart { id, .. }, StreamEvent::MessageStop] if id == "msg_multiline"
    ));
}

#[tokio::test]
async fn anthropic_stream_closes_cleanly_after_message_stop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_complete\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n",
            "event: message_stop\n",
            "data: {}\n\n"
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(body.as_bytes()).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [StreamEvent::MessageStart { id, .. }, StreamEvent::MessageStop] if id == "msg_complete"
    ));
}

#[tokio::test]
async fn anthropic_stream_ignores_frames_after_message_stop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_complete\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n",
            "event: message_stop\n",
            "data: {}\n\n",
            "event: message_start\n",
            "data: not-json\n\n"
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(body.as_bytes()).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [StreamEvent::MessageStart { id, .. }, StreamEvent::MessageStop] if id == "msg_complete"
    ));
}

#[tokio::test]
async fn anthropic_stream_stops_after_provider_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let body = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"retry later\"}}\n\n",
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_later\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n"
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(body.as_bytes()).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [StreamEvent::Error { error_type, message }] if error_type == "overloaded_error" && message == "retry later"
    ));
}

#[tokio::test]
async fn anthropic_stream_stops_after_parse_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let malformed = b"event: message_start\ndata: not-json\n\n";
        let valid = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_later\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n";
        let body = [malformed.as_slice(), valid.as_slice()].concat();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(&body).await.expect("write body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [StreamEvent::Error { error_type, .. }] if error_type == "parse_error"
    ));
}

#[tokio::test]
async fn anthropic_stream_emits_network_error_after_body_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!(
        "http://{}/v1/messages",
        listener.local_addr().expect("address")
    );

    tokio::spawn(async move {
        let mut socket = accept_request(&listener).await;
        let event = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"usage\":{}}}\n\n";
        let claimed_length = event.len() + 100;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {claimed_length}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write headers");
        socket.write_all(event).await.expect("write partial body");
    });

    let mut stream = client(url)
        .stream_message(MessageRequest::default())
        .await
        .expect("start stream");
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }

    assert!(matches!(
        events.as_slice(),
        [
            StreamEvent::MessageStart { .. },
            StreamEvent::Error { error_type, .. }
        ] if error_type == "network"
    ));
}
