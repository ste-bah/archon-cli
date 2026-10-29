use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_request(socket: &mut TcpStream) -> bool {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        if socket.read(&mut byte).await.unwrap() == 0 {
            return false;
        }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return true;
        }
    }
}

async fn answer(socket: &mut TcpStream) {
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
        .await
        .unwrap();
}

/// Exercise the actual client's connection policy, independently of endpoint
/// authentication. A fresh socket for consecutive calls is stronger than a
/// reset conditional on a clock jump: it covers every wall/monotonic delta.
pub(crate) async fn assert_fresh_connections(http: reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        assert!(read_request(&mut first).await);
        answer(&mut first).await;
        // The peer has silently lost this connection after the first reply.
        // If the client reuses it, drop it without a response.
        let mut reused = false;
        let mut next = tokio::select! {
            request = read_request(&mut first) => {
                if request {
                    reused = true;
                    drop(first);
                    return reused;
                }
                listener.accept().await.unwrap().0
            }
            accepted = listener.accept() => accepted.unwrap().0,
        };
        assert!(read_request(&mut next).await);
        answer(&mut next).await;
        reused
    });
    http.post(&url).send().await.unwrap().text().await.unwrap();
    // No monotonic delay is required to detect stale socket reuse. In
    // particular an arbitrarily large intervening wall-clock jump is safe.
    let response = http.post(&url).send().await;
    let reused = server.await.unwrap();
    assert!(
        !reused,
        "the next attempt reused the dead keep-alive connection"
    );
    assert_eq!(response.unwrap().status(), reqwest::StatusCode::OK);
}

pub(crate) async fn error_response(status: u16, wait: &str) -> reqwest::Response {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status} AC-X-001\r\nRetry-After: {wait}\r\nContent-Length: 8\r\nConnection: close\r\n\r\nAC-X-001"
    );
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert!(read_request(&mut socket).await);
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    crate::transport_http::client_builder()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap()
}

pub(crate) async fn truncated_response_then_recovery(
    body: String,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for truncated in [true, false] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
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
            let response = if truncated {
                "HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n{".to_string()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (url, server)
}
