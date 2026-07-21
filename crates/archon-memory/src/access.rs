//! Unified memory access: direct (server) or remote (client).
//!
//! [`open_memory`] is the main entry point. It decides whether this process
//! should own the CozoDB instance (first session) or connect to an existing
//! server (subsequent sessions).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::client::MemoryClient;
use crate::graph::MemoryGraph;
use crate::server::MemoryServer;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome};

mod client_impl;
#[cfg(test)]
mod tests;
mod trait_impl;

// ── MemoryAccess enum ──────────────────────────────────────────

/// Unified access to the memory graph, either direct or via TCP.
///
/// Both variants implement [`MemoryTrait`], so callers can use
/// `MemoryAccess` polymorphically without caring which mode is active.
pub enum MemoryAccess {
    /// This process owns the CozoDB instance and TCP server.
    Direct {
        graph: Arc<MemoryGraph>,
        _server_handle: tokio::task::JoinHandle<()>,
    },
    /// This process connects to an existing memory server.
    Remote(MemoryClient),
}

// ── trait ──────────────────────────────────────────────────────

/// Object-safe trait covering all 13 public [`MemoryGraph`] operations.
///
/// Both [`MemoryGraph`] (direct) and [`MemoryClient`] (remote via TCP)
/// implement this trait, so callers can be polymorphic.
pub trait MemoryTrait: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError>;

    #[allow(clippy::too_many_arguments)]
    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError>;

    #[allow(clippy::too_many_arguments)]
    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError>;

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError>;

    /// Retrieve a single memory without updating its access metadata.
    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError>;

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError>;

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError>;

    /// Atomically compare the authoritative importance to a prior snapshot,
    /// replace the trend tag, and return the reconciled row.
    fn reconcile_importance_trend(
        &self,
        id: &str,
        previous_importance: f64,
    ) -> Result<Memory, MemoryError> {
        let memory = self.inspect_memory(id)?;
        let mut tags: Vec<_> = memory
            .tags
            .iter()
            .filter(|tag| !tag.starts_with("trend:"))
            .cloned()
            .collect();
        let trend = match memory.importance.total_cmp(&previous_importance) {
            std::cmp::Ordering::Greater => "trend:rising",
            std::cmp::Ordering::Equal => "trend:stable",
            std::cmp::Ordering::Less => "trend:declining",
        };
        tags.push(trend.to_string());
        self.update_memory(id, None, Some(&tags))?;
        self.inspect_memory(id)
    }

    /// Return whether this memory has durably recorded an importance delta
    /// for the given immutable provenance identifier.
    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError>;

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError>;

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError>;

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError>;

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError>;

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError>;

    fn memory_count(&self) -> Result<usize, MemoryError>;

    fn clear_all(&self) -> Result<usize, MemoryError>;

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError>;
}

// ── factory ────────────────────────────────────────────────────

/// Open the singleton memory system.
///
/// `data_dir` is the directory where `memory.port`, `memory.lock`, and
/// `memory.db` live (typically `~/.local/share/archon`).
///
/// 1. If a port file exists and the server responds to ping, connect as client.
/// 2. Otherwise, acquire a file lock, open CozoDB, start the TCP server, and
///    become the server process.
///
/// # Runtime behavior
///
/// This async entry point requires a Tokio runtime while opening the memory
/// service. The synchronous [`MemoryTrait`] methods on a returned `Remote`
/// variant bridge calls from multi-threaded, current-thread, or no-runtime
/// contexts.
pub async fn open_memory(data_dir: &Path) -> Result<MemoryAccess, MemoryError> {
    open_memory_with_db_path(data_dir, &data_dir.join("memory.db")).await
}

pub fn default_memory_data_dir() -> PathBuf {
    std::env::var("ARCHON_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("archon")
        })
}

pub fn resolve_memory_paths(configured: Option<&str>) -> (PathBuf, PathBuf) {
    if let Some(value) = configured.filter(|value| !value.trim().is_empty()) {
        let path = PathBuf::from(value);
        if path.extension().is_some_and(|ext| ext == "db") {
            let data_dir = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(default_memory_data_dir);
            return (data_dir, path);
        }
        return (path.clone(), path.join("memory.db"));
    }
    let data_dir = default_memory_data_dir();
    let db_path = data_dir.join("memory.db");
    (data_dir, db_path)
}

/// Open the singleton memory system with an explicit Cozo database path.
///
/// `data_dir` still owns the singleton coordination files (`memory.port` and
/// `memory.lock`). `db_path` owns the actual Cozo memory graph.
pub async fn open_memory_with_db_path(
    data_dir: &Path,
    db_path: &Path,
) -> Result<MemoryAccess, MemoryError> {
    std::fs::create_dir_all(data_dir)?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let port_file = data_dir.join("memory.port");
    let lock_file = data_dir.join("memory.lock");

    // Fast path: if port file exists, try to connect.
    if let Some(access) = try_connect_existing(&port_file).await {
        return Ok(access);
    }

    // Slow path: acquire lock, re-check, maybe start server.
    let lock_fd = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_file)?;

    let mut lock = fd_lock::RwLock::new(lock_fd);
    let _guard = lock
        .try_write()
        .map_err(|e| MemoryError::Database(format!("failed to acquire memory lock: {e}")))?;

    // Re-check after acquiring lock (another process may have started).
    if let Some(access) = try_connect_existing(&port_file).await {
        return Ok(access);
    }

    // We are the server. Open CozoDB and start listening.
    info!("starting memory server");
    let graph = MemoryGraph::open(db_path)?;
    let graph = Arc::new(graph);
    let (port, handle) = MemoryServer::start(Arc::clone(&graph), port_file).await?;
    info!(port, "memory server started");

    // Lock is released here (guard drops), but the server keeps running.
    Ok(MemoryAccess::Direct {
        graph,
        _server_handle: handle,
    })
}

/// Try to connect to an existing server from the port file.
/// Returns `None` if the port file doesn't exist, is unparseable,
/// or the server doesn't respond to ping (stale).
async fn try_connect_existing(port_file: &Path) -> Option<MemoryAccess> {
    let contents = std::fs::read_to_string(port_file).ok()?;
    let port: u16 = contents.trim().parse().ok()?;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().ok()?;

    match MemoryClient::connect(addr).await {
        Ok(client) => match client.ping().await {
            Ok(()) => {
                debug!(port, "connected to existing memory server");
                Some(MemoryAccess::Remote(client))
            }
            Err(e) => {
                warn!(port, error = %e, "server ping failed, cleaning stale port file");
                let _ = std::fs::remove_file(port_file);
                None
            }
        },
        Err(e) => {
            debug!(port, error = %e, "cannot connect to memory server, cleaning stale port file");
            let _ = std::fs::remove_file(port_file);
            None
        }
    }
}
