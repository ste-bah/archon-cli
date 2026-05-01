//! Unified memory access: direct (server) or remote (client).
//!
//! [`open_memory`] is the main entry point. It decides whether this process
//! should own the CozoDB instance (first session) or connect to an existing
//! server (subsequent sessions).

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::client::MemoryClient;
use crate::graph::MemoryGraph;
use crate::server::MemoryServer;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

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

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError>;

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError>;

    fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError>;

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

// ── MemoryGraph impl ───────────────────────────────────────────

impl MemoryTrait for MemoryGraph {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        MemoryGraph::store_memory(
            self,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        MemoryGraph::get_memory(self, id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        MemoryGraph::update_memory(self, id, content, tags)
    }

    fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        MemoryGraph::update_importance(self, id, importance)
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        MemoryGraph::delete_memory(self, id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        MemoryGraph::create_relationship(self, from_id, to_id, rel_type, context, strength)
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::recall_memories(self, query, limit)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::search_memories(self, filter)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::list_recent(self, limit)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        MemoryGraph::memory_count(self)
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        MemoryGraph::clear_all(self)
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::get_related_memories(self, id, depth)
    }
}

// ── MemoryClient impl ──────────────────────────────────────────

/// Helper: run an async call on the current tokio runtime from a sync context.
///
/// Uses `block_in_place` + `block_on` so the current thread yields to the
/// scheduler while blocking.
///
/// # Panics
///
/// Panics if called outside a **multi-threaded** tokio runtime (i.e. there is
/// no current `Handle`, or the runtime is single-threaded and
/// `block_in_place` is unsupported).
fn block_on_async<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

/// [`MemoryTrait`] implementation that bridges async [`MemoryClient`] calls
/// into the synchronous trait interface via [`block_on_async`].
///
/// # Runtime requirement
///
/// `MemoryClient` requires a **multi-threaded** tokio runtime.  Calling any
/// [`MemoryTrait`] method on a `MemoryClient` from outside a tokio runtime
/// (or from a single-threaded runtime) will panic.
impl MemoryTrait for MemoryClient {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        let result = block_on_async(self.call(
            "store_memory",
            serde_json::json!({
                "content": content,
                "title": title,
                "memory_type": format!("{memory_type}"),
                "importance": importance,
                "tags": tags,
                "source_type": source_type,
                "project_path": project_path,
            }),
        ))?;
        result
            .as_str()
            .map(String::from)
            .ok_or_else(|| MemoryError::Database("expected string id".to_string()))
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call("get_memory", serde_json::json!({"id": id})))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        let mut params = serde_json::json!({"id": id});
        if let Some(c) = content {
            params["content"] = serde_json::Value::String(c.to_string());
        }
        if let Some(t) = tags {
            params["tags"] = serde_json::to_value(t)?;
        }
        block_on_async(self.call("update_memory", params))?;
        Ok(())
    }

    fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        block_on_async(self.call(
            "update_importance",
            serde_json::json!({"id": id, "importance": importance}),
        ))?;
        Ok(())
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        block_on_async(self.call("delete_memory", serde_json::json!({"id": id})))?;
        Ok(())
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        block_on_async(self.call(
            "create_relationship",
            serde_json::json!({
                "from_id": from_id,
                "to_id": to_id,
                "rel_type": format!("{rel_type}"),
                "context": context,
                "strength": strength,
            }),
        ))?;
        Ok(())
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call(
            "recall_memories",
            serde_json::json!({"query": query, "limit": limit}),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        let filter_val = serde_json::to_value(filter)?;
        let result = block_on_async(
            self.call("search_memories", serde_json::json!({"filter": filter_val})),
        )?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call("list_recent", serde_json::json!({"limit": limit})))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        let result = block_on_async(self.call("memory_count", serde_json::json!({})))?;
        result
            .as_u64()
            .map(|v| v as usize)
            .ok_or_else(|| MemoryError::Database("expected integer count".to_string()))
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        let result = block_on_async(self.call("clear_all", serde_json::json!({})))?;
        result
            .as_u64()
            .map(|v| v as usize)
            .ok_or_else(|| MemoryError::Database("expected integer count".to_string()))
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call(
            "get_related_memories",
            serde_json::json!({"id": id, "depth": depth}),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }
}

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

impl MemoryAccess {
    /// Return the underlying [`MemoryGraph`] if this is a `Direct` access,
    /// or `None` for a `Remote` client.
    pub fn graph(&self) -> Option<&Arc<MemoryGraph>> {
        match self {
            Self::Direct { graph, .. } => Some(graph),
            Self::Remote(_) => None,
        }
    }
}

impl MemoryTrait for MemoryAccess {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.store_memory(
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
            Self::Remote(client) => client.store_memory(
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
        }
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.get_memory(id),
            Self::Remote(client) => client.get_memory(id),
        }
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.update_memory(id, content, tags),
            Self::Remote(client) => client.update_memory(id, content, tags),
        }
    }

    fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.update_importance(id, importance),
            Self::Remote(client) => client.update_importance(id, importance),
        }
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.delete_memory(id),
            Self::Remote(client) => client.delete_memory(id),
        }
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => {
                graph.create_relationship(from_id, to_id, rel_type, context, strength)
            }
            Self::Remote(client) => {
                client.create_relationship(from_id, to_id, rel_type, context, strength)
            }
        }
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.recall_memories(query, limit),
            Self::Remote(client) => client.recall_memories(query, limit),
        }
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.search_memories(filter),
            Self::Remote(client) => client.search_memories(filter),
        }
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.list_recent(limit),
            Self::Remote(client) => client.list_recent(limit),
        }
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.memory_count(),
            Self::Remote(client) => client.memory_count(),
        }
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.clear_all(),
            Self::Remote(client) => client.clear_all(),
        }
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.get_related_memories(id, depth),
            Self::Remote(client) => client.get_related_memories(id, depth),
        }
    }
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
/// # Runtime requirement
///
/// Must be called from within a **multi-threaded** tokio runtime. The
/// `Remote` variant returned uses [`MemoryClient`], whose [`MemoryTrait`]
/// methods call [`block_on_async`] internally — this will panic outside a
/// tokio context.
pub async fn open_memory(data_dir: &Path) -> Result<MemoryAccess, MemoryError> {
    std::fs::create_dir_all(data_dir)?;

    let port_file = data_dir.join("memory.port");
    let lock_file = data_dir.join("memory.lock");
    let db_path = data_dir.join("memory.db");

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
    let graph = MemoryGraph::open(&db_path)?;
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

// ── tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::MemoryGraph;
    use crate::types::MemoryType;

    /// Verify that `Arc<dyn MemoryTrait>` works with a concrete `MemoryGraph`,
    /// proving the trait is object-safe and usable polymorphically.
    #[test]
    fn arc_dyn_memory_trait_store_and_recall() {
        let graph = MemoryGraph::in_memory().expect("in-memory graph");
        let mem: Arc<dyn MemoryTrait> = Arc::new(graph);

        let id = mem
            .store_memory(
                "Rust async uses tokio runtime",
                "async note",
                MemoryType::Fact,
                0.8,
                &["rust".into(), "async".into()],
                "test",
                "/test",
            )
            .expect("store_memory via trait object");

        let recalled = mem
            .recall_memories("tokio", 5)
            .expect("recall via trait object");
        assert!(!recalled.is_empty(), "should recall at least one memory");
        assert_eq!(recalled[0].id, id);
    }

    /// Verify that `MemoryAccess` (the enum) works as `Arc<dyn MemoryTrait>`.
    #[test]
    fn memory_access_as_dyn_trait() {
        let graph = MemoryGraph::in_memory().expect("in-memory graph");
        let access = MemoryAccess::Direct {
            graph: Arc::new(graph),
            _server_handle: tokio::runtime::Runtime::new().unwrap().spawn(async {}),
        };
        let mem: Arc<dyn MemoryTrait> = Arc::new(access);

        let _id = mem
            .store_memory(
                "test content",
                "title",
                MemoryType::Fact,
                0.5,
                &[],
                "test",
                "",
            )
            .expect("store via MemoryAccess trait object");

        assert_eq!(mem.memory_count().expect("count"), 1);
    }
}
