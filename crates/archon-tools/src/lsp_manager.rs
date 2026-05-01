//! LSP server manager for TASK-CLI-313.
//!
//! Detects language servers from project markers, loads custom config,
//! and manages lazy initialization of the `LspClient`.

use std::path::{Path, PathBuf};

use crate::lsp_client::LspClient;

// ---------------------------------------------------------------------------
// LspConfig — custom server paths from .archon/lsp-config.json
// ---------------------------------------------------------------------------

/// Per-language server config from `.archon/lsp-config.json`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct LspServerConfig {
    /// Server binary path (overrides auto-detection).
    pub command: Option<String>,
    /// Extra CLI arguments passed to the server.
    pub args: Option<Vec<String>>,
    /// Language ID this config applies to (e.g. "rust", "typescript").
    pub language_id: Option<String>,
}

/// Top-level `.archon/lsp-config.json` structure.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct LspConfig {
    pub servers: Vec<LspServerConfig>,
}

impl LspConfig {
    pub fn load(project_root: &Path) -> Self {
        let new_path = project_root.join(".archon").join("lsp-config.json");
        if let Ok(s) = std::fs::read_to_string(&new_path)
            && let Ok(cfg) = serde_json::from_str(&s) {
                return cfg;
            }
        let old_path = project_root.join(".claude").join("lsp-config.json");
        if let Ok(s) = std::fs::read_to_string(&old_path)
            && let Ok(cfg) = serde_json::from_str(&s) {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_path.display(),
                    new_path.display()
                );
                return cfg;
            }
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// LspServerManager
// ---------------------------------------------------------------------------

/// Manages the lifecycle of an LSP server connection.
///
/// Lazy initialization: the connection is established on first tool use.
pub struct LspServerManager {
    project_root: PathBuf,
    config: LspConfig,
    client: Option<LspClient>,
}

impl LspServerManager {
    /// Create a new manager. Pass `Some(config)` to override file-based config.
    pub fn new(project_root: PathBuf, config: Option<LspConfig>) -> Self {
        let config = config.unwrap_or_else(|| LspConfig::load(&project_root));
        Self {
            project_root,
            config,
            client: None,
        }
    }

    /// Returns true if an LSP connection is currently active.
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Get a mutable reference to the active client, if any.
    pub fn client_mut(&mut self) -> Option<&mut LspClient> {
        self.client.as_mut()
    }

    /// Detect the appropriate language server for this project.
    ///
    /// Returns `(binary_name, extra_args)` or `None` if no server is detected.
    ///
    /// Detection order:
    /// 1. Custom config from `.archon/lsp-config.json` (first matching entry)
    /// 2. `Cargo.toml` → rust-analyzer
    /// 3. `package.json` → typescript-language-server
    /// 4. `pyproject.toml` or `setup.py` → pylsp (prefer) or pyright
    pub fn detect_language_server(&self) -> Option<(String, Vec<String>)> {
        // 1. Custom config overrides
        if let Some(server) = self.config.servers.first()
            && let Some(cmd) = &server.command {
                let args = server.args.clone().unwrap_or_default();
                return Some((cmd.clone(), args));
            }

        // 2. Cargo.toml → rust-analyzer
        if self.project_root.join("Cargo.toml").exists() {
            return Some(("rust-analyzer".to_string(), vec![]));
        }

        // 3. package.json → typescript-language-server
        if self.project_root.join("package.json").exists() {
            return Some((
                "typescript-language-server".to_string(),
                vec!["--stdio".to_string()],
            ));
        }

        // 4. pyproject.toml or setup.py → pylsp
        if self.project_root.join("pyproject.toml").exists()
            || self.project_root.join("setup.py").exists()
        {
            return Some(("pylsp".to_string(), vec![]));
        }

        None
    }

    /// Initialize the LSP client (async, lazy — called on first tool use).
    ///
    /// Returns an error if no language server is detected or if the server
    /// binary is missing.
    pub async fn ensure_connected(&mut self) -> Result<(), crate::lsp_client::LspError> {
        if self.client.is_some() {
            return Ok(());
        }

        let (binary, args) = self
            .detect_language_server()
            .ok_or(crate::lsp_client::LspError::NoServerDetected)?;

        let root_uri = lsp_types::Url::from_file_path(&self.project_root)
            .map_err(|_| crate::lsp_client::LspError::InvalidProjectRoot)?;

        let client = LspClient::connect(
            &binary,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            root_uri,
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(10),
        )
        .await?;

        self.client = Some(client);
        Ok(())
    }

    /// Shut down the active LSP client gracefully.
    pub async fn shutdown(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.shutdown().await;
        }
    }
}
