//! Stdio transport layer for MCP servers.
//!
//! Wraps `rmcp`'s `TokioChildProcess` transport, adding environment
//! variable injection and structured error handling.

use std::collections::HashMap;

use rmcp::transport::child_process::{ConfigureCommandExt, TokioChildProcess};
use tokio::process::Command;

use crate::types::{McpError, ServerConfig};

/// Create a `TokioChildProcess` transport from a [`ServerConfig`].
///
/// The child process is spawned with piped stdin/stdout for JSON-RPC
/// communication and inherits stderr for diagnostic logging.
pub fn spawn_transport(config: &ServerConfig) -> Result<TokioChildProcess, McpError> {
    let env_clone: HashMap<String, String> = config.env.clone();
    let args_clone: Vec<String> = config.args.clone();

    let cmd = Command::new(&config.command).configure(|cmd| {
        cmd.args(&args_clone);
        for (k, v) in &env_clone {
            cmd.env(k, v);
        }
    });

    TokioChildProcess::new(cmd).map_err(|e| {
        McpError::Transport(format!(
            "failed to spawn '{}': {}",
            config.command, e
        ))
    })
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
        };
        // echo exits immediately, but spawn itself should succeed
        let result = spawn_transport(&config);
        assert!(result.is_ok());
    }
}
