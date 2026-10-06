//! Stdio transport layer for MCP servers.
//!
//! Wraps `rmcp`'s `TokioChildProcess` transport, adding environment
//! variable injection and structured error handling.

use std::collections::HashMap;

use rmcp::transport::child_process::{ConfigureCommandExt, TokioChildProcess};

use crate::types::{McpError, ServerConfig};

/// Create a `TokioChildProcess` transport from a [`ServerConfig`].
///
/// The child process is spawned with piped stdin/stdout for JSON-RPC
/// communication. Stderr is piped through the redacted diagnostic log sink.
pub fn spawn_transport(config: &ServerConfig) -> Result<TokioChildProcess, McpError> {
    let secrets = config.configured_secrets();
    secrets.register();
    let env_clone: HashMap<String, String> = config.env.clone();
    let args_clone: Vec<String> = config.args.clone();

    let cmd = archon_shell::spawn::tokio_command(&config.command).configure(|cmd| {
        cmd.args(&args_clone);
        for (k, v) in &env_clone {
            cmd.env(k, v);
        }
    });

    let (transport, stderr) = TokioChildProcess::builder(cmd)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            McpError::Transport(format!("failed to spawn '{}': {}", config.command, e))
                .redacted(&secrets)
        })?;
    if let Some(stderr) = stderr {
        use tracing::instrument::WithSubscriber;
        let server = archon_observability::redaction::redact_text(&config.name);
        tokio::spawn(drain_stderr(stderr, server).with_current_subscriber());
    }
    Ok(transport)
}

/// Bound both buffering and each logged diagnostic, while retaining the pipe
/// until EOF. Oversized lines are omitted as a whole so a truncated credential
/// cannot leak a prefix. Each chunk still drains through the next newline.
const MAX_STDERR_LINE_BYTES: usize = 4096;

pub(crate) async fn drain_stderr(reader: impl tokio::io::AsyncRead + Unpin, server: String) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::with_capacity(MAX_STDERR_LINE_BYTES + 1);
    let mut discarding = false;
    loop {
        bytes.clear();
        let result = (&mut reader)
            .take((MAX_STDERR_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await;
        match result {
            Ok(count) => {
                discarding |= count > MAX_STDERR_LINE_BYTES;
                if count <= MAX_STDERR_LINE_BYTES || bytes.last() == Some(&b'\n') {
                    if discarding {
                        log_stderr(&server, "[stderr line omitted: exceeds 4096 bytes]");
                    } else if count > 0 {
                        log_stderr(
                            &server,
                            String::from_utf8_lossy(&bytes).trim_end_matches(['\n', '\r']),
                        );
                    }
                    discarding = false;
                }
                if count == 0 {
                    break;
                }
            }
            Err(error) => {
                // Don't emit a partial value after a failed read. Resume draining
                // after transient errors, with backoff to avoid a busy loop.
                discarding |= !bytes.is_empty();
                tracing::warn!(%server, error = %archon_observability::redaction::redact_text(&error.to_string()), "MCP stderr read failed");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

fn log_stderr(server: &str, line: &str) {
    let mut diagnostic = archon_observability::redaction::redact_text(line);
    // Lossy UTF-8 decoding can expand bytes; cap only after complete redaction.
    if diagnostic.len() > MAX_STDERR_LINE_BYTES {
        let mut end = MAX_STDERR_LINE_BYTES;
        while !diagnostic.is_char_boundary(end) {
            end -= 1;
        }
        diagnostic.truncate(end);
    }
    tracing::info!(%server, %diagnostic, "MCP server stderr");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_transport_with_valid_command() {
        let config = ServerConfig {
            name: "test".into(),
            command: "cat".into(),
            args: vec![],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
            allow_insecure_ws: false,
            tool_policy: Default::default(),
        };
        // `cat` with piped stdin will block waiting for input, which is fine
        // for a transport — we just check it spawns successfully
        let result = spawn_transport(&config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn spawn_transport_with_bad_command() {
        let config = ServerConfig {
            name: "bad".into(),
            command: "/nonexistent/binary/path".into(),
            args: vec![],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
            allow_insecure_ws: false,
            tool_policy: Default::default(),
        };
        let result = spawn_transport(&config);
        assert!(result.is_err());
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("/nonexistent/binary/path"));
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn spawn_transport_with_env_vars() {
        let mut env = HashMap::new();
        env.insert("MY_CUSTOM_VAR".into(), "custom_value".into());

        let config = ServerConfig {
            name: "env-test".into(),
            command: "cat".into(),
            args: vec![],
            env,
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
            allow_insecure_ws: false,
            tool_policy: Default::default(),
        };
        let result = spawn_transport(&config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn spawn_transport_with_args() {
        let config = ServerConfig {
            name: "args-test".into(),
            command: "echo".into(),
            args: vec!["hello".into(), "world".into()],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
            allow_insecure_ws: false,
            tool_policy: Default::default(),
        };
        // echo exits immediately, but spawn itself should succeed
        let result = spawn_transport(&config);
        assert!(result.is_ok());
    }
}
