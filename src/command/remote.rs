//! Remote (SSH/WS) and Serve command handlers.
//! Extracted from main.rs to reduce main.rs from 1032 to < 500 lines.

use std::path::PathBuf;

use archon_core::config::ArchonConfig;

use crate::cli_args::ServeArgs;
use crate::cli_args::{Cli, Commands, RemoteAction};

/// Handle `/archon remote` and `/archon serve` subcommands.
pub async fn handle_remote_command(
    cli: &Cli,
    config: &ArchonConfig,
) -> std::result::Result<(), anyhow::Error> {
    // Remote
    if let Some(Commands::Remote { action }) = &cli.command {
        match action {
            RemoteAction::Ssh {
                target,
                command,
                port,
                key,
            } => {
                use archon_core::remote::{
                    RemoteTransport, SshConnectionConfig, SyncMode, protocol::AgentMessage,
                    ssh::SshTransport,
                };
                let (user, host) = target
                    .split_once('@')
                    .map(|(u, h)| (u.to_string(), h.to_string()))
                    .unwrap_or_else(|| ("root".to_string(), target.clone()));
                let remote_session_id = cli
                    .session_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                tracing::info!(
                    "remote ssh: user={user} host={host} port={port} session_tag={}",
                    archon_core::remote::session_tag(&remote_session_id)
                );
                println!(
                    "Remote SSH: connecting to {user}@{host}:{port} (session {remote_session_id})"
                );
                tracing::info!(
                    "remote ssh: agent_forwarding={} (from config.remote.ssh.agent_forwarding)",
                    config.remote.ssh.agent_forwarding
                );
                let ssh_cfg = SshConnectionConfig {
                    host: host.clone(),
                    port: *port,
                    user: user.clone(),
                    key_file: key.clone(),
                    agent_forwarding: config.remote.ssh.agent_forwarding,
                    session_id: remote_session_id.clone(),
                    sync_mode: match config.remote.sync_mode.as_str() {
                        "auto" => SyncMode::Auto,
                        _ => SyncMode::Manual,
                    },
                };
                let session = match SshTransport.connect(&ssh_cfg).await {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("SSH connection failed: {e}");
                        std::process::exit(1);
                    }
                };
                println!("Connected. Session: {}", session.session_id);
                if let Some(cmd) = command {
                    let msg = AgentMessage::UserMessage {
                        content: cmd.clone(),
                    };
                    if let Err(e) = session.send(&msg).await {
                        eprintln!("SSH send failed: {e}");
                        let _ = session.disconnect().await;
                        std::process::exit(1);
                    }
                    match session.recv().await {
                        Ok(AgentMessage::AssistantMessage { content }) => println!("{content}"),
                        Ok(AgentMessage::Error { message }) => {
                            eprintln!("remote error: {message}");
                            let _ =
                                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                                    session.disconnect().await
                                })
                                .await;
                            std::process::exit(1);
                        }
                        Ok(other) => println!("{other:?}"),
                        Err(e) => {
                            eprintln!("SSH recv failed: {e}");
                            std::process::exit(1);
                        }
                    }
                } else if let Err(e) = session.disconnect().await {
                    eprintln!("SSH disconnect failed: {e}");
                    std::process::exit(1);
                }
            }
            RemoteAction::Ws { url, token } => {
                use archon_core::remote::websocket::{WsConnectionConfig, WsTransport};
                let remote_session_id = cli
                    .session_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let cfg = WsConnectionConfig {
                    url: url.clone(),
                    token: token.clone().unwrap_or_default(),
                    reconnect: false,
                    max_reconnect_attempts: 0,
                    session_id: remote_session_id.clone(),
                };
                // `url` may carry `?token=`; print only its origin.
                let shown_url = archon_core::url_redact::redact_url(url);
                trace_ws_connect(&shown_url, &remote_session_id);
                println!(
                    "Remote WebSocket: connecting to {shown_url} (session {remote_session_id})"
                );
                match WsTransport.connect_ws(&cfg).await {
                    Ok(session) => println!("Connected. Session: {}", session.session_id),
                    Err(e) => {
                        eprintln!("WebSocket connection failed: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        return Ok(());
    }

    // Serve
    if let Some(Commands::Serve(ServeArgs { port, token_path })) = &cli.command {
        use archon_core::remote::{
            server::WebSocketServer,
            websocket::{IdeHandlerFn, WsServerConfig},
        };
        use archon_sdk::ide::handler::IdeProtocolHandler;
        use std::sync::Arc;
        use tokio::sync::Mutex;
        let mut srv_cfg = WsServerConfig {
            port: *port,
            tls_cert: config.ws_remote.tls_cert.as_ref().map(PathBuf::from),
            tls_key: config.ws_remote.tls_key.as_ref().map(PathBuf::from),
            ..Default::default()
        };
        if let Some(tp) = token_path
            && let Ok(tok) = std::fs::read_to_string(tp)
        {
            srv_cfg.token = Some(tok.trim().to_string());
        }
        // Handshake-only on purpose: `/ws/ide` is request/response with no way
        // to push a frame, so an agent behind it could run a turn that the
        // client would never see. `IdeProtocolHandler::new` refuses every
        // agent-backed method explicitly rather than pretending to queue it —
        // the IDE extension's agent transport is `archon ide-stdio`.
        let ide_proto = IdeProtocolHandler::new(env!("CARGO_PKG_VERSION"));
        let ide_handler: IdeHandlerFn = Arc::new(Mutex::new(Box::new({
            let mut h = ide_proto;
            move |req: &str| h.handle(req)
        })));
        srv_cfg.ide_handler = Some(ide_handler);
        match WebSocketServer::new(srv_cfg).await {
            Ok(server) => {
                if let Err(e) = server.run().await {
                    eprintln!("server error: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("failed to start server: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    Ok(())
}

fn trace_ws_connect(shown_url: &str, session_id: &str) {
    tracing::info!(
        "remote ws: connecting to {shown_url} session_tag={}",
        archon_core::remote::session_tag(session_id)
    );
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn websocket_connection_log_hides_id_and_includes_tag() {
        let raw_id = "AC-X-001";
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(move || CaptureWriter(Arc::clone(&writer_output)))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            super::trace_ws_connect("wss://example.invalid", raw_id);
        });

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(
            !output.contains(raw_id),
            "captured log exposed raw id: {output}"
        );
        assert!(output.contains("session_tag="), "captured log: {output}");
    }
}
