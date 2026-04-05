//! TCP client that connects to a running [`MemoryServer`](crate::server::MemoryServer).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

use crate::protocol::{make_request, parse_response};
use crate::types::MemoryError;

/// A JSON-RPC client connected to the singleton memory server.
pub struct MemoryClient {
    reader: Mutex<BufReader<OwnedReadHalf>>,
    writer: Mutex<OwnedWriteHalf>,
    next_id: AtomicU64,
}

impl MemoryClient {
    /// Connect to a running memory server with a 2-second timeout.
    pub async fn connect(addr: SocketAddr) -> Result<Self, MemoryError> {
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .map_err(|_| MemoryError::Database(format!("connection to {addr} timed out")))??;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: Mutex::new(BufReader::new(read_half)),
            writer: Mutex::new(write_half),
            next_id: AtomicU64::new(1),
        })
    }

    /// Send a JSON-RPC call and wait for the response.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, MemoryError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = make_request(id, method, params);

        // Hold both locks for the duration of the call to ensure
        // request/response pairing.
        let mut writer = self.writer.lock().await;
        let mut reader = self.reader.lock().await;

        writer.write_all(req.as_bytes()).await?;

        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(MemoryError::Database(
                "server closed connection".to_string(),
            ));
        }

        let resp = parse_response(&line)?;
        if resp.id != id {
            return Err(MemoryError::Database(format!(
                "response id mismatch: expected {id}, got {}",
                resp.id
            )));
        }

        if let Some(err) = resp.error {
            return Err(MemoryError::Database(err.message));
        }

        Ok(resp.result.unwrap_or(Value::Null))
    }

    /// Ping the server to verify liveness.
    pub async fn ping(&self) -> Result<(), MemoryError> {
        let result = self.call("ping", serde_json::json!({})).await?;
        if result.as_str() == Some("pong") {
            Ok(())
        } else {
            Err(MemoryError::Database(format!(
                "unexpected ping response: {result}"
            )))
        }
    }
}
