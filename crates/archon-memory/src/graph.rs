use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use tracing::debug;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::search;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

/// Helper to convert CozoDB errors into MemoryError.
fn db_err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError::Database(e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

/// Minimum content length required to generate an embedding.
const MIN_EMBED_CHARS: usize = 10;

/// CozoDB-backed memory graph.
pub struct MemoryGraph {
    pub(crate) db: DbInstance,
    embedding_provider: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    hybrid_alpha: RwLock<f32>,
}

impl MemoryGraph {
    // ── constructors ──────────────────────────────────────────

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

    // ── schema ────────────────────────────────────────────────

    fn init_schema(&self) -> Result<(), MemoryError> {
        self.db
            .run_script(
                ":create memories {
                    id: String
                    =>
                    content: String,
                    title: String,
                    memory_type: String,
                    importance: Float,
                    tags: String,
                    source_type: String,
                    project_path: String,
                    created_at: String,
                    updated_at: String,
                    access_count: Int,
                    last_accessed: String
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;

        self.db
            .run_script(
                ":create relationships {
                    from_id: String,
                    to_id: String,
                    rel_type: String
                    =>
                    context: String,
                    strength: Float,
                    created_at: String
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;

        Ok(())
    }

    /// Borrow the underlying CozoDB instance.
    ///
    /// Exposed for subsystems that need direct database access (e.g. vector
    /// search schema management).  Prefer higher-level `MemoryGraph` methods
    /// when possible.
    pub fn db(&self) -> &DbInstance {
        &self.db
    }

    // ── embedding provider ────────────────────────────────────

    /// Attach a semantic embedding provider for hybrid search.
    ///
    /// This also initialises the vector schema in CozoDB and sets the
    /// hybrid alpha blend factor.  Takes `&self` (not `&mut self`) so it
    /// can be called through `Arc<MemoryGraph>`.
    pub fn set_embedding_provider(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Result<(), MemoryError> {
        crate::vector_search::init_embedding_schema(&self.db, provider.dimensions())?;
        let mut guard = self
            .embedding_provider
            .write()
            .map_err(|e| MemoryError::Database(format!("embedding provider lock poisoned: {e}")))?;
        *guard = Some(provider);
        Ok(())
    }

    /// Set the hybrid alpha for keyword/vector blending.
    pub fn set_hybrid_alpha(&self, alpha: f32) {
        if let Ok(mut guard) = self.hybrid_alpha.write() {
            *guard = alpha.clamp(0.0, 1.0);
        }
    }

    /// Read the current hybrid alpha value.
    fn read_hybrid_alpha(&self) -> f32 {
        self.hybrid_alpha.read().map(|g| *g).unwrap_or(0.3)
    }

    /// Embed content and store the vector alongside the memory.
    fn embed_and_store(&self, memory_id: &str, content: &str) {
        if content.len() < MIN_EMBED_CHARS {
            return;
        }
        let provider = match self.embedding_provider.read() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };
        if let Some(ref provider) = provider {
            match provider.embed(&[content.to_string()]) {
                Ok(vecs) if !vecs.is_empty() => {
                    if let Err(e) = crate::vector_search::store_embedding(
                        &self.db,
                        memory_id,
                        &vecs[0],
                        "auto",
                        provider.dimensions(),
                    ) {
                        tracing::warn!(memory_id, "failed to store embedding: {e}");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(memory_id, "failed to generate embedding: {e}");
                }
            }
        }
    }

    // ── CRUD ──────────────────────────────────────────────────

    /// Store a new memory and return its UUID.
    pub fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id.as_str()));
        params.insert("content".to_string(), DataValue::from(content));
        params.insert("title".to_string(), DataValue::from(title));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(importance));
        params.insert("tags".to_string(), DataValue::from(tags_json.as_str()));
        params.insert("source_type".to_string(), DataValue::from(source_type));
        params.insert("project_path".to_string(), DataValue::from(project_path));
        params.insert("created_at".to_string(), DataValue::from(now.as_str()));
        params.insert("updated_at".to_string(), DataValue::from(""));
        params.insert("access_count".to_string(), DataValue::from(0i64));
        params.insert("last_accessed".to_string(), DataValue::from(""));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $access_count, $last_accessed
                ]]
                :put memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        debug!(id = %id, "stored memory");
        self.embed_and_store(&id, content);
        Ok(id)
    }

    /// Retrieve a single memory by id and bump its access stats.
    pub fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let mem = self.read_memory(id)?;

        // Bump access count and last_accessed.
        let now = Utc::now().to_rfc3339();
        let new_count = mem.access_count + 1;

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(mem.content.as_str()));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(mem.importance));
        params.insert(
            "tags".to_string(),
            DataValue::from(serde_json::to_string(&mem.tags).map_err(MemoryError::from)?),
        );
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        let updated_str = mem.updated_at.map(|dt| dt.to_rfc3339()).unwrap_or_default();
        params.insert("updated_at".to_string(), DataValue::from(updated_str));
        params.insert("new_count".to_string(), DataValue::from(new_count as i64));
        params.insert("now".to_string(), DataValue::from(now.as_str()));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $new_count, $now
                ]]
                :put memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(Memory {
            access_count: new_count,
            last_accessed: Some(
                chrono::DateTime::parse_from_rfc3339(&now)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            ),
            ..mem
        })
    }

    /// Update a memory's content and/or tags.
    pub fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        let mem = self.read_memory(id)?;
        let now = Utc::now().to_rfc3339();

        let new_content = content.unwrap_or(&mem.content);
        let new_tags = if let Some(t) = tags {
            serde_json::to_string(t)?
        } else {
            serde_json::to_string(&mem.tags)?
        };

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(new_content));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(mem.importance));
        params.insert("tags".to_string(), DataValue::from(new_tags));
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        params.insert("updated_at".to_string(), DataValue::from(now));
        params.insert(
            "access_count".to_string(),
            DataValue::from(mem.access_count as i64),
        );
        let la = mem
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        params.insert("last_accessed".to_string(), DataValue::from(la));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $access_count, $last_accessed
                ]]
                :put memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Update a memory's importance score.
    pub fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        let mem = self.read_memory(id)?;
        let now = Utc::now().to_rfc3339();

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(mem.content.as_str()));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(importance));
        params.insert(
            "tags".to_string(),
            DataValue::from(serde_json::to_string(&mem.tags).map_err(MemoryError::from)?),
        );
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        params.insert("updated_at".to_string(), DataValue::from(now));
        params.insert(
            "access_count".to_string(),
            DataValue::from(mem.access_count as i64),
        );
        let la = mem
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        params.insert("last_accessed".to_string(), DataValue::from(la));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $access_count, $last_accessed
                ]]
                :put memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Delete a memory, its relationships, and its embedding (if any).
    pub fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        // Check it exists first
        self.read_memory(id)?;

        // Delete the embedding if vector search is initialised
        let has_provider = self
            .embedding_provider
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if has_provider && let Err(e) = crate::vector_search::delete_embedding(&self.db, id) {
            tracing::warn!(id, "failed to delete embedding: {e}");
        }

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));

        // Find all relationships involving this memory
        let rels = self
            .db
            .run_script(
                "?[from_id, to_id, rel_type] := *relationships{from_id, to_id, rel_type}, from_id = $id
                 ?[from_id, to_id, rel_type] := *relationships{from_id, to_id, rel_type}, to_id = $id",
                params.clone(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        // Delete each relationship individually
        for row in &rels.rows {
            let mut rp = BTreeMap::new();
            rp.insert("from_id".to_string(), row[0].clone());
            rp.insert("to_id".to_string(), row[1].clone());
            rp.insert("rel_type".to_string(), row[2].clone());
            self.db
                .run_script(
                    "?[from_id, to_id, rel_type] <- [[$from_id, $to_id, $rel_type]]
                     :rm relationships {from_id, to_id, rel_type}",
                    rp,
                    ScriptMutability::Mutable,
                )
                .map_err(db_err)?;
        }

        // Delete the memory itself
        self.db
            .run_script(
                "?[id] <- [[$id]] :rm memories {id}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    // ── relationships ─────────────────────────────────────────

    /// Create a directed relationship between two memories.
    pub fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        let now = Utc::now().to_rfc3339();
        let ctx = context.unwrap_or("");

        let mut params = BTreeMap::new();
        params.insert("from_id".to_string(), DataValue::from(from_id));
        params.insert("to_id".to_string(), DataValue::from(to_id));
        params.insert(
            "rel_type".to_string(),
            DataValue::from(rel_type.to_string()),
        );
        params.insert("context".to_string(), DataValue::from(ctx));
        params.insert("strength".to_string(), DataValue::from(strength));
        params.insert("created_at".to_string(), DataValue::from(now));

        self.db
            .run_script(
                "?[from_id, to_id, rel_type, context, strength, created_at] <- [[
                    $from_id, $to_id, $rel_type, $context, $strength, $created_at
                ]]
                :put relationships {from_id, to_id, rel_type => context, strength, created_at}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    // ── search / recall ───────────────────────────────────────

    /// Recall memories by relevance.
    ///
    /// When an embedding provider is attached, uses hybrid (keyword + vector)
    /// search. Otherwise falls back to keyword-only search.
    pub fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let provider = self
            .embedding_provider
            .read()
            .map_err(|e| MemoryError::Database(format!("embedding provider lock poisoned: {e}")))?;
        if let Some(ref provider) = *provider {
            crate::hybrid_search::hybrid_search(
                &self.db,
                query,
                provider.as_ref(),
                self.read_hybrid_alpha(),
                limit,
            )
        } else {
            search::recall(&self.db, query, limit)
        }
    }

    /// Structured search with filters.
    pub fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        search::search(&self.db, filter)
    }

    /// List the most recently created memories (up to `limit`).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let all = read_all_memories(&self.db)?;
        let mut memories: Vec<Memory> = all
            .into_iter()
            .filter_map(|raw| raw_to_memory(raw).ok())
            .collect();
        // Sort descending by created_at (newest first)
        memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        memories.truncate(limit);
        Ok(memories)
    }

    /// Return the total number of memories in the graph.
    pub fn memory_count(&self) -> Result<usize, MemoryError> {
        let result = self
            .db
            .run_script(
                "?[count(id)] := *memories{id}",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let count = result
            .rows
            .first()
            .and_then(|row| row[0].get_int())
            .unwrap_or(0);
        Ok(count as usize)
    }

    /// Delete all memories and relationships from the graph.
    pub fn clear_all(&self) -> Result<usize, MemoryError> {
        let count = self.memory_count()?;
        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] :=
                    *memories{id, content, title, memory_type, importance, tags,
                              source_type, project_path, created_at, updated_at,
                              access_count, last_accessed}
                :rm memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        self.db
            .run_script(
                "?[from_id, to_id, rel_type, context, strength, created_at] :=
                    *relationships{from_id, to_id, rel_type, context, strength, created_at}
                :rm relationships {
                    from_id, to_id, rel_type => context, strength, created_at
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(count)
    }

    // ── graph traversal ───────────────────────────────────────

    /// Recursive BFS traversal up to `depth` hops from a starting memory.
    /// Returns all reachable memories (excluding the start node).
    pub fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        if depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(id.to_string());
        let mut frontier: Vec<String> = vec![id.to_string()];

        for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<String> = Vec::new();
            for node in &frontier {
                let neighbours = self.direct_neighbours(node)?;
                for n in neighbours {
                    if visited.insert(n.clone()) {
                        next_frontier.push(n);
                    }
                }
            }
            frontier = next_frontier;
        }

        visited.remove(id);

        let mut results = Vec::with_capacity(visited.len());
        for mem_id in &visited {
            match self.read_memory(mem_id) {
                Ok(m) => results.push(m),
                Err(MemoryError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(results)
    }

    /// Return ids of all direct neighbours (both directions).
    fn direct_neighbours(&self, id: &str) -> Result<Vec<String>, MemoryError> {
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));

        let result = self
            .db
            .run_script(
                "?[neighbour] := *relationships{from_id, to_id}, from_id = $id, neighbour = to_id
                 ?[neighbour] := *relationships{from_id, to_id}, to_id = $id, neighbour = from_id",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut ids = Vec::new();
        for row in &result.rows {
            if let Some(s) = row[0].get_str() {
                ids.push(s.to_string());
            }
        }
        Ok(ids)
    }

    // ── internal read helper ─────────────────────────────────

    /// Read a single memory by id without bumping access stats.
    pub(crate) fn read_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        row_to_memory(&self.db, id)
    }
}

// ── helpers ──────────────────────────────────────────────────

fn extract_str(val: &DataValue) -> String {
    val.get_str().unwrap_or("").to_string()
}

fn extract_f64(val: &DataValue) -> f64 {
    val.get_float().unwrap_or(0.0)
}

fn extract_i64(val: &DataValue) -> i64 {
    val.get_int().unwrap_or(0)
}

/// Read a single memory row by id.
pub(crate) fn row_to_memory(db: &DbInstance, id: &str) -> Result<Memory, MemoryError> {
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(id));

    let result = db
        .run_script(
            "?[id, content, title, memory_type, importance, tags,
              source_type, project_path, created_at, updated_at,
              access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags,
                          source_type, project_path, created_at, updated_at,
                          access_count, last_accessed},
                id = $id",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    if result.rows.is_empty() {
        return Err(MemoryError::NotFound(id.to_string()));
    }

    let row = &result.rows[0];
    let raw = RawRow {
        id: extract_str(&row[0]),
        content: extract_str(&row[1]),
        title: extract_str(&row[2]),
        memory_type: extract_str(&row[3]),
        importance: extract_f64(&row[4]),
        tags: extract_str(&row[5]),
        source_type: extract_str(&row[6]),
        project_path: extract_str(&row[7]),
        created_at: extract_str(&row[8]),
        updated_at: extract_str(&row[9]),
        access_count: extract_i64(&row[10]),
        last_accessed: extract_str(&row[11]),
    };

    raw_to_memory(raw)
}

pub(crate) struct RawRow {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) title: String,
    pub(crate) memory_type: String,
    pub(crate) importance: f64,
    pub(crate) tags: String,
    pub(crate) source_type: String,
    pub(crate) project_path: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) access_count: i64,
    pub(crate) last_accessed: String,
}

pub(crate) fn raw_to_memory(raw: RawRow) -> Result<Memory, MemoryError> {
    let memory_type = MemoryType::from_str_opt(&raw.memory_type)
        .ok_or_else(|| MemoryError::InvalidType(raw.memory_type.clone()))?;
    let tags: Vec<String> = serde_json::from_str(&raw.tags)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&raw.created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(&raw.updated_at)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();
    let last_accessed = chrono::DateTime::parse_from_rfc3339(&raw.last_accessed)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();

    Ok(Memory {
        id: raw.id,
        content: raw.content,
        title: raw.title,
        memory_type,
        importance: raw.importance,
        tags,
        source_type: raw.source_type,
        project_path: raw.project_path,
        created_at,
        updated_at,
        access_count: raw.access_count as u64,
        last_accessed,
    })
}

/// Read all memory rows from the database.
pub(crate) fn read_all_memories(db: &DbInstance) -> Result<Vec<RawRow>, MemoryError> {
    let result = db
        .run_script(
            "?[id, content, title, memory_type, importance, tags,
              source_type, project_path, created_at, updated_at,
              access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags,
                          source_type, project_path, created_at, updated_at,
                          access_count, last_accessed}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    let mut rows = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        rows.push(RawRow {
            id: extract_str(&row[0]),
            content: extract_str(&row[1]),
            title: extract_str(&row[2]),
            memory_type: extract_str(&row[3]),
            importance: extract_f64(&row[4]),
            tags: extract_str(&row[5]),
            source_type: extract_str(&row[6]),
            project_path: extract_str(&row[7]),
            created_at: extract_str(&row[8]),
            updated_at: extract_str(&row[9]),
            access_count: extract_i64(&row[10]),
            last_accessed: extract_str(&row[11]),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryType, RelType, SearchFilter};

    fn make_graph() -> MemoryGraph {
        MemoryGraph::in_memory().expect("failed to create in-memory graph")
    }

    #[test]
    fn store_and_get() {
        let g = make_graph();
        let id = g
            .store_memory(
                "Rust is great",
                "rust fact",
                MemoryType::Fact,
                0.8,
                &["rust".into(), "lang".into()],
                "manual",
                "/tmp",
            )
            .expect("store failed");

        let m = g.get_memory(&id).expect("get failed");
        assert_eq!(m.content, "Rust is great");
        assert_eq!(m.memory_type, MemoryType::Fact);
        assert_eq!(m.tags, vec!["rust", "lang"]);
        assert_eq!(m.access_count, 1);
    }

    #[test]
    fn get_missing_returns_not_found() {
        let g = make_graph();
        let err = g.get_memory("nonexistent").unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[test]
    fn update_memory_content_and_tags() {
        let g = make_graph();
        let id = g
            .store_memory("old", "", MemoryType::Decision, 0.5, &[], "m", "")
            .expect("store failed");
        g.update_memory(&id, Some("new"), Some(&["tag1".into()]))
            .expect("update failed");
        let m = g.get_memory(&id).expect("get failed");
        assert_eq!(m.content, "new");
        assert_eq!(m.tags, vec!["tag1"]);
        assert!(m.updated_at.is_some());
    }

    #[test]
    fn update_nonexistent_errors() {
        let g = make_graph();
        let err = g.update_memory("nope", Some("x"), None).unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[test]
    fn delete_memory_and_relationships() {
        let g = make_graph();
        let a = g
            .store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let b = g
            .store_memory("b", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        g.create_relationship(&a, &b, RelType::RelatedTo, None, 1.0)
            .expect("rel failed");
        g.delete_memory(&a).expect("delete failed");

        assert!(g.get_memory(&b).is_ok());
        assert!(g.get_memory(&a).is_err());
        let related = g.get_related_memories(&b, 1).expect("traversal failed");
        assert!(related.is_empty());
    }

    #[test]
    fn delete_nonexistent_errors() {
        let g = make_graph();
        let err = g.delete_memory("nope").unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[test]
    fn graph_traversal_depth() {
        let g = make_graph();
        let a = g
            .store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let b = g
            .store_memory("b", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let c = g
            .store_memory("c", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let d = g
            .store_memory("d", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        g.create_relationship(&a, &b, RelType::RelatedTo, None, 1.0)
            .expect("rel failed");
        g.create_relationship(&b, &c, RelType::CausedBy, None, 1.0)
            .expect("rel failed");
        g.create_relationship(&c, &d, RelType::DerivedFrom, None, 1.0)
            .expect("rel failed");

        let r1 = g.get_related_memories(&a, 1).expect("traversal failed");
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].id, b);

        let r2 = g.get_related_memories(&a, 2).expect("traversal failed");
        assert_eq!(r2.len(), 2);

        let r3 = g.get_related_memories(&a, 3).expect("traversal failed");
        assert_eq!(r3.len(), 3);

        let r0 = g.get_related_memories(&a, 0).expect("traversal failed");
        assert!(r0.is_empty());
    }

    #[test]
    fn recall_basic() {
        let g = make_graph();
        g.store_memory(
            "Rust memory safety is important",
            "safety",
            MemoryType::Fact,
            0.9,
            &["rust".into()],
            "manual",
            "",
        )
        .expect("store failed");
        g.store_memory(
            "Python is dynamically typed",
            "python",
            MemoryType::Fact,
            0.5,
            &["python".into()],
            "manual",
            "",
        )
        .expect("store failed");

        let results = g.recall_memories("rust safety", 10).expect("recall failed");
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn recall_empty_query_returns_empty() {
        let g = make_graph();
        g.store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let r = g.recall_memories("", 10).expect("recall failed");
        assert!(r.is_empty());
    }

    #[test]
    fn search_by_type() {
        let g = make_graph();
        g.store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        g.store_memory("b", "", MemoryType::Decision, 0.5, &[], "m", "")
            .expect("store failed");

        let filter = SearchFilter {
            memory_type: Some(MemoryType::Decision),
            ..Default::default()
        };
        let results = g.search_memories(&filter).expect("search failed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "b");
    }

    #[test]
    fn search_by_tags_any() {
        let g = make_graph();
        g.store_memory("a", "", MemoryType::Fact, 0.5, &["x".into()], "m", "")
            .expect("store failed");
        g.store_memory("b", "", MemoryType::Fact, 0.5, &["y".into()], "m", "")
            .expect("store failed");
        g.store_memory(
            "c",
            "",
            MemoryType::Fact,
            0.5,
            &["x".into(), "y".into()],
            "m",
            "",
        )
        .expect("store failed");

        let filter = SearchFilter {
            tags: vec!["x".into()],
            require_all_tags: false,
            ..Default::default()
        };
        let results = g.search_memories(&filter).expect("search failed");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_by_tags_all() {
        let g = make_graph();
        g.store_memory("a", "", MemoryType::Fact, 0.5, &["x".into()], "m", "")
            .expect("store failed");
        g.store_memory(
            "c",
            "",
            MemoryType::Fact,
            0.5,
            &["x".into(), "y".into()],
            "m",
            "",
        )
        .expect("store failed");

        let filter = SearchFilter {
            tags: vec!["x".into(), "y".into()],
            require_all_tags: true,
            ..Default::default()
        };
        let results = g.search_memories(&filter).expect("search failed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "c");
    }

    #[test]
    fn search_by_text() {
        let g = make_graph();
        g.store_memory("alpha beta", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        g.store_memory("gamma delta", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        let filter = SearchFilter {
            text: Some("beta".into()),
            ..Default::default()
        };
        let results = g.search_memories(&filter).expect("search failed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("beta"));
    }

    #[test]
    fn empty_graph_operations() {
        let g = make_graph();
        let r = g.recall_memories("anything", 10).expect("recall failed");
        assert!(r.is_empty());
        let s = g
            .search_memories(&SearchFilter::default())
            .expect("search failed");
        assert!(s.is_empty());
        let rel = g
            .get_related_memories("nonexistent", 3)
            .expect("traversal failed");
        assert!(rel.is_empty());
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir failed");
        let db_path = dir.path().join("test.db");

        let id = {
            let g = MemoryGraph::open(&db_path).expect("open failed");
            g.store_memory("persist me", "", MemoryType::Rule, 0.9, &[], "m", "")
                .expect("store failed")
        };

        let g = MemoryGraph::open(&db_path).expect("reopen failed");
        let m = g.get_memory(&id).expect("get failed");
        assert_eq!(m.content, "persist me");
    }
}
