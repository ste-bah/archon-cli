use crate::provider::LlmError;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub(crate) async fn endpoint(reply: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 8192];
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0, "provider fixture must receive request bytes");
        if !reply.is_empty() {
            socket.write_all(reply).await.unwrap();
        }
    });
    (url, peer)
}
pub(crate) fn assert_transient(error: LlmError, hint: Option<u64>) {
    if let Some(hint) = hint {
        assert_eq!(error.retry_after_hint(), Some(hint), "{error:?}");
    }
    assert!(
        matches!(
            error,
            LlmError::Transport(_)
                | LlmError::Server {
                    status: 408 | 500..=599,
                    ..
                }
                | LlmError::Overloaded { .. }
                | LlmError::RateLimited { .. }
        ),
        "untyped operational fault: {error:?}"
    );
}
pub(crate) const STATUSES: &[(&[u8], Option<u64>)] = &[
    (b"HTTP/1.1 408 Request Timeout\r\nContent-Length: 2\r\nRetry-After: 7\r\nConnection: close\r\n\r\n{}", Some(7)),
    (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nRetry-After: 7\r\nConnection: close\r\n\r\n{}", Some(7)),
];
