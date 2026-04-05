use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use russh::ChannelMsg;
use tokio::sync::Mutex;

use super::{
    protocol::AgentMessage, RemoteSession, RemoteSessionInner, RemoteTransport, SshConnectionConfig,
};

pub struct SshTransport;

// ---------------------------------------------------------------------------
// Known-hosts TOFU helpers
// ---------------------------------------------------------------------------

fn known_hosts_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()))
        .join("archon")
        .join("known_hosts.json")
}

fn load_known_hosts(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_known_hosts(path: &Path, hosts: &HashMap<String, String>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(hosts)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// SSH client handler with TOFU host key verification
// ---------------------------------------------------------------------------

struct SshClientHandler {
    /// "host:port" key used to look up / store the known fingerprint.
    host_key_str: String,
    /// Path to the TOFU known-hosts JSON file.
    known_hosts_path: PathBuf,
}

impl SshClientHandler {
    fn new(host: &str, port: u16) -> Self {
        Self {
            host_key_str: format!("{host}:{port}"),
            known_hosts_path: known_hosts_path(),
        }
    }
}

impl russh::client::Handler for SshClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();

        let mut hosts = load_known_hosts(&self.known_hosts_path);

        match hosts.get(&self.host_key_str) {
            Some(known_fp) => {
                if known_fp == &fingerprint {
                    tracing::info!(
                        "ssh: host key verified for {} ({})",
                        self.host_key_str,
                        fingerprint
                    );
                    Ok(true)
                } else {
                    anyhow::bail!(
                        "ssh: HOST KEY MISMATCH for {} — stored fingerprint {} does not match \
                         server fingerprint {}. If the host key changed legitimately, remove the \
                         entry for {} from {:?}",
                        self.host_key_str,
                        known_fp,
                        fingerprint,
                        self.host_key_str,
                        self.known_hosts_path,
                    )
                }
            }
            None => {
                tracing::warn!(
                    "ssh: new host {} — trusting key on first use (TOFU): {}",
                    self.host_key_str,
                    fingerprint
                );
                hosts.insert(self.host_key_str.clone(), fingerprint);
                if let Err(e) = save_known_hosts(&self.known_hosts_path, &hosts) {
                    tracing::warn!("ssh: failed to persist known_hosts: {e}");
                }
                Ok(true)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SSH session inner
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct SshSessionInner {
    channel: Mutex<russh::Channel<russh::client::Msg>>,
    read_buf: Mutex<String>,
}

/// Maximum bytes buffered before a newline before treating the connection as broken.
#[allow(dead_code)]
const MAX_READ_BUF: usize = 4 * 1024 * 1024; // 4 MiB

#[async_trait]
impl RemoteSessionInner for SshSessionInner {
    async fn send(&self, message: &AgentMessage) -> anyhow::Result<()> {
        let line = message.to_json_line()?;
        let channel = self.channel.lock().await;
        channel
            .data(line.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("ssh: write failed: {e}"))?;
        Ok(())
    }

    async fn recv(&self) -> anyhow::Result<AgentMessage> {
        loop {
            {
                let mut buf = self.read_buf.lock().await;
                if let Some(pos) = buf.find('\n') {
                    let line = buf[..=pos].to_string();
                    buf.drain(..=pos);
                    return AgentMessage::from_json_line(&line);
                }
            }

            let msg = {
                let mut channel = self.channel.lock().await;
                channel.wait().await
            };

            match msg {
                Some(ChannelMsg::Data { data }) => {
                    let chunk = std::str::from_utf8(&data)
                        .map_err(|e| anyhow::anyhow!("ssh: non-UTF8 data: {e}"))?;
                    let mut buf = self.read_buf.lock().await;
                    if buf.len() + chunk.len() > MAX_READ_BUF {
                        anyhow::bail!(
                            "ssh: read buffer exceeded {MAX_READ_BUF} bytes without newline — \
                             possible malicious or misbehaving remote"
                        );
                    }
                    buf.push_str(chunk);
                }
                Some(ChannelMsg::Eof) | None => {
                    anyhow::bail!("ssh: remote channel closed (EOF)");
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    anyhow::bail!("ssh: remote exited with status {exit_status}");
                }
                Some(_) => {
                    tracing::debug!("ssh: ignoring channel message");
                }
            }
        }
    }

    async fn disconnect(&self) -> anyhow::Result<()> {
        let channel = self.channel.lock().await;
        channel
            .eof()
            .await
            .map_err(|e| anyhow::anyhow!("ssh: eof failed: {e}"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SshTransport
// ---------------------------------------------------------------------------

#[async_trait]
impl RemoteTransport for SshTransport {
    async fn connect(&self, config: &SshConnectionConfig) -> anyhow::Result<RemoteSession> {
        tracing::info!(
            "ssh: connecting to {}@{}:{}",
            config.user,
            config.host,
            config.port
        );

        let ssh_config = Arc::new(russh::client::Config::default());
        let handler = SshClientHandler::new(&config.host, config.port);
        let mut session =
            russh::client::connect(ssh_config, (config.host.as_str(), config.port), handler)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "ssh: connection to {}:{} failed: {e}",
                        config.host,
                        config.port
                    )
                })?;

        authenticate(&mut session, config).await?;

        let channel = session
            .channel_open_session()
            .await
            .map_err(|e| anyhow::anyhow!("ssh: channel_open_session failed: {e}"))?;

        let remote_cmd = format!(
            "archon --headless --session-id {}",
            shell_escape(&config.session_id)
        );
        channel
            .exec(true, remote_cmd.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("ssh: exec failed: {e}"))?;

        let inner = SshSessionInner {
            channel: Mutex::new(channel),
            read_buf: Mutex::new(String::new()),
        };

        tracing::info!(
            "ssh: session established session_id={}",
            config.session_id
        );

        Ok(RemoteSession {
            session_id: config.session_id.clone(),
            inner: Box::new(inner),
        })
    }
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

async fn authenticate(
    session: &mut russh::client::Handle<SshClientHandler>,
    config: &SshConnectionConfig,
) -> anyhow::Result<()> {
    if let Some(ref key_path) = config.key_file {
        tracing::info!("ssh: authenticating with key file");
        let key = russh::keys::load_secret_key(key_path, None)
            .map_err(|e| anyhow::anyhow!("ssh: failed to load key: {e}"))?;
        let key_with_hash = russh::keys::key::PrivateKeyWithHashAlg::new(Arc::new(key), None);
        let result = session
            .authenticate_publickey(&config.user, key_with_hash)
            .await
            .map_err(|e| anyhow::anyhow!("ssh: public key auth failed: {e}"))?;
        if !result.success() {
            anyhow::bail!(
                "ssh: public key auth rejected for user '{}'",
                config.user
            );
        }
        tracing::info!("ssh: authenticated via public key");
        return Ok(());
    }

    // No key file — try SSH agent if available (respects agent_forwarding config
    // and SSH_AUTH_SOCK environment variable).
    let agent_available =
        config.agent_forwarding || std::env::var("SSH_AUTH_SOCK").is_ok();

    if agent_available {
        tracing::info!("ssh: attempting authentication via SSH agent");
        match try_agent_auth(session, &config.user).await {
            Ok(true) => return Ok(()),
            Ok(false) => tracing::info!("ssh: no agent identities accepted; falling back"),
            Err(e) => tracing::warn!("ssh: SSH agent error: {e}; falling back"),
        }
    }

    // Last resort: empty-password probe (useful for dev containers / test VMs).
    tracing::info!("ssh: attempting password auth (no key or agent configured)");
    let result = session
        .authenticate_password(&config.user, "")
        .await
        .map_err(|e| anyhow::anyhow!("ssh: password auth failed: {e}"))?;
    if !result.success() {
        anyhow::bail!(
            "ssh: authentication rejected for user '{}' — \
             provide --key, configure an SSH agent (SSH_AUTH_SOCK), \
             or set agent_forwarding = true in config",
            config.user
        );
    }
    Ok(())
}

/// Try each identity offered by the SSH agent in turn.
/// Returns `Ok(true)` if any identity was accepted.
async fn try_agent_auth(
    session: &mut russh::client::Handle<SshClientHandler>,
    user: &str,
) -> anyhow::Result<bool> {
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(|e| anyhow::anyhow!("ssh: connect to SSH agent: {e}"))?;

    let identities = agent
        .request_identities()
        .await
        .map_err(|e| anyhow::anyhow!("ssh: request identities from SSH agent: {e}"))?;

    if identities.is_empty() {
        tracing::warn!("ssh: SSH agent has no identities");
        return Ok(false);
    }

    for identity in &identities {
        let public_key = identity.public_key().into_owned();
        match session
            .authenticate_publickey_with(user, public_key, None, &mut agent)
            .await
        {
            Ok(result) if result.success() => {
                tracing::info!("ssh: authenticated via SSH agent identity");
                return Ok(true);
            }
            Ok(_) => tracing::debug!("ssh: agent identity not accepted, trying next"),
            Err(e) => tracing::debug!("ssh: agent auth attempt error: {e}"),
        }
    }

    Ok(false)
}

// ---------------------------------------------------------------------------
// Shell escaping
// ---------------------------------------------------------------------------

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
