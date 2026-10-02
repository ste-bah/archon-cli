//! Loopback tests for the SSH transport against an in-process russh server.
//!
//! The server plays a hostile peer: after accepting archon's exec channel it
//! tries to open every kind of channel archon never asked for, then answers on
//! the real channel. Archon must refuse all of them and still read only its
//! own channel.

use std::collections::HashMap;
use std::sync::Arc;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth};
use russh::{Channel, ChannelId, ChannelOpenFailure};
use tokio::sync::oneshot;

use super::{SshClientHandler, connect_with_handler, load_known_hosts};
use crate::remote::protocol::AgentMessage;
use crate::remote::{SshConnectionConfig, SyncMode};

type ProbeResults = Vec<(&'static str, Option<ChannelOpenFailure>)>;

struct HostileServer {
    client_key: PublicKey,
    exec_tx: Option<oneshot::Sender<String>>,
    probe_tx: Option<oneshot::Sender<ProbeResults>>,
}

fn open_failure<T>(res: Result<T, russh::Error>) -> Option<ChannelOpenFailure> {
    match res {
        Err(russh::Error::ChannelOpenFailure(reason)) => Some(reason),
        _ => None,
    }
}

/// Try to open every server-initiated channel type towards the client.
async fn probe_server_channels(h: &server::Handle) -> ProbeResults {
    vec![
        ("session", open_failure(h.channel_open_session().await)),
        (
            "x11",
            open_failure(h.channel_open_x11("127.0.0.1", 6000).await),
        ),
        (
            "direct-tcpip",
            open_failure(
                h.channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 5000)
                    .await,
            ),
        ),
        (
            "forwarded-tcpip",
            open_failure(
                h.channel_open_forwarded_tcpip("127.0.0.1", 22, "127.0.0.1", 5000)
                    .await,
            ),
        ),
        (
            "direct-streamlocal",
            open_failure(h.channel_open_direct_streamlocal("/nonexistent").await),
        ),
        (
            "forwarded-streamlocal",
            open_failure(h.channel_open_forwarded_streamlocal("/nonexistent").await),
        ),
        ("agent-forward", open_failure(h.channel_open_agent().await)),
    ]
}

impl server::Handler for HostileServer {
    type Error = russh::Error;

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if key.key_data() == self.client_key.key_data() {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        if let Some(tx) = self.exec_tx.take() {
            let _ = tx.send(String::from_utf8_lossy(data).into_owned());
        }
        let handle = session.handle();
        let probe_tx = self.probe_tx.take();
        tokio::spawn(async move {
            let results = probe_server_channels(&handle).await;
            let pong = AgentMessage::Pong.to_json_line().unwrap_or_default();
            let _ = handle.data(channel, pong.into_bytes()).await;
            if let Some(tx) = probe_tx {
                let _ = tx.send(results);
            }
        });
        Ok(())
    }
}

fn key_from_seed(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

struct Harness {
    port: u16,
    exec_rx: oneshot::Receiver<String>,
    probe_rx: oneshot::Receiver<ProbeResults>,
    dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let client_key = key_from_seed(7);
        let key_pem = client_key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode client key");
        std::fs::write(dir.path().join("id_ed25519"), key_pem.as_bytes()).expect("write key");

        let mut config = server::Config::default();
        config.keys.push(key_from_seed(9));
        config.auth_rejection_time = std::time::Duration::from_millis(10);
        let config = Arc::new(config);

        let (exec_tx, exec_rx) = oneshot::channel();
        let (probe_tx, probe_rx) = oneshot::channel();
        let handler = HostileServer {
            client_key: client_key.public_key().clone(),
            exec_tx: Some(exec_tx),
            probe_tx: Some(probe_tx),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            if let Ok(running) = server::run_stream(config, socket, handler).await {
                let _ = running.await;
            }
        });

        Self {
            port,
            exec_rx,
            probe_rx,
            dir,
        }
    }

    fn known_hosts(&self) -> std::path::PathBuf {
        self.dir.path().join("known_hosts.json")
    }

    fn config(&self) -> SshConnectionConfig {
        SshConnectionConfig {
            host: "127.0.0.1".into(),
            port: self.port,
            user: "archon-test".into(),
            key_file: Some(self.dir.path().join("id_ed25519")),
            agent_forwarding: false,
            session_id: "sess-235".into(),
            sync_mode: SyncMode::Manual,
        }
    }

    fn handler(&self) -> SshClientHandler {
        SshClientHandler::with_known_hosts("127.0.0.1", self.port, self.known_hosts())
    }
}

#[tokio::test]
async fn server_initiated_channels_are_rejected_and_own_channel_still_reads() {
    let h = Harness::start().await;
    let config = h.config();
    let known_hosts = h.known_hosts();
    let session = connect_with_handler(&config, h.handler())
        .await
        .expect("connect over loopback");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), session.recv())
        .await
        .expect("recv timed out")
        .expect("recv");
    assert!(matches!(msg, AgentMessage::Pong), "got {msg:?}");

    let exec = h.exec_rx.await.expect("exec seen");
    assert_eq!(exec, "archon --headless --session-id 'sess-235'");

    let probes = h.probe_rx.await.expect("probe results");
    assert_eq!(probes.len(), 7);
    for (kind, outcome) in probes {
        assert_eq!(
            outcome,
            Some(ChannelOpenFailure::AdministrativelyProhibited),
            "server-initiated {kind} channel was not rejected"
        );
    }

    let hosts = load_known_hosts(&known_hosts);
    let pinned = hosts
        .get(&format!("127.0.0.1:{}", config.port))
        .expect("TOFU pinned the host key");
    let expected = key_from_seed(9)
        .public_key()
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();
    assert_eq!(pinned, &expected);
}

#[tokio::test]
async fn pinned_host_key_mismatch_refuses_connection() {
    let h = Harness::start().await;
    let mut hosts = HashMap::new();
    hosts.insert(
        format!("127.0.0.1:{}", h.port),
        "SHA256:not-the-real-fingerprint".to_string(),
    );
    super::save_known_hosts(&h.known_hosts(), &hosts).expect("seed known hosts");

    let err = connect_with_handler(&h.config(), h.handler())
        .await
        .expect_err("mismatched host key must be refused");
    assert!(err.to_string().contains("HOST KEY MISMATCH"), "{err}");
}
