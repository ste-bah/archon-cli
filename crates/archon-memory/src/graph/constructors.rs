use std::path::Path;
use std::sync::RwLock;

use cozo::DbInstance;

use super::MemoryGraph;
use super::helpers::db_err;
#[cfg(unix)]
use super::helpers::secure_file_permissions;
use crate::types::MemoryError;

impl MemoryGraph {
    // -- constructors ------------------------------------------

    /// Open (or create) the graph at the default location
    /// `~/.local/share/archon/memory.db`.
    pub fn open_default() -> Result<Self, MemoryError> {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("archon");
        std::fs::create_dir_all(&dir)?;
        Self::open(dir.join("memory.db"))
    }

    /// Open (or create) the graph at an explicit path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MemoryError> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let db = DbInstance::new("sqlite", &path_str, "").map_err(db_err)?;

        #[cfg(unix)]
        secure_file_permissions(path.as_ref())?;

        let graph = Self {
            db,
            embedding_provider: RwLock::new(None),
            hybrid_alpha: RwLock::new(0.3),
        };
        graph.init_schema()?;
        Ok(graph)
    }

    /// Create an in-memory graph (useful for tests).
    pub fn in_memory() -> Result<Self, MemoryError> {
        let db = DbInstance::new("mem", "", "").map_err(db_err)?;
        let graph = Self {
            db,
            embedding_provider: RwLock::new(None),
            hybrid_alpha: RwLock::new(0.3),
        };
        graph.init_schema()?;
        Ok(graph)
    }

    /// Borrow the underlying CozoDB instance.
    ///
    /// Exposed for subsystems that need direct database access (e.g. vector
    /// search schema management).  Prefer higher-level `MemoryGraph` methods
    /// when possible.
    pub fn db(&self) -> &DbInstance {
        &self.db
    }
}
