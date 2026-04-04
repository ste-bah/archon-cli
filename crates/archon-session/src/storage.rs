use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session database error: {0}")]
    DbError(String),

    #[error("session I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("session not found: {0}")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// Session metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: String,
    pub last_active: String,
    pub working_directory: String,
    pub git_branch: Option<String>,
    pub model: String,
    pub message_count: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub schema_version: u32,
    /// Human-readable session name (optional).
    pub name: Option<String>,
    /// ID of the session this was forked from (optional).
    pub parent_session_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn db_err(e: impl std::fmt::Display) -> SessionError {
    SessionError::DbError(e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

fn extract_str(val: &DataValue) -> String {
    val.get_str().unwrap_or("").to_string()
}

fn extract_i64(val: &DataValue) -> i64 {
    val.get_int().unwrap_or(0)
}

fn extract_f64(val: &DataValue) -> f64 {
    val.get_float().unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Session store
// ---------------------------------------------------------------------------

pub struct SessionStore {
    db: DbInstance,
}

impl SessionStore {
    /// Open (or create) the session database at the given path.
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let path_str = path.to_string_lossy().to_string();
        let db = DbInstance::new("sqlite", &path_str, "").map_err(db_err)?;

        let store = Self { db };
        store.init_schema()?;
        Ok(store)
    }

    /// Open the default session database.
    pub fn open_default() -> Result<Self, SessionError> {
        Self::open(&default_db_path())
    }

    fn init_schema(&self) -> Result<(), SessionError> {
        self.db
            .run_script(
                ":create sessions {
                    id: String
                    =>
                    created_at: String,
                    last_active: String,
                    working_directory: String,
                    git_branch: String,
                    model: String,
                    message_count: Int,
                    total_tokens: Int,
                    total_cost: Float,
                    schema_version: Int
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
                ":create messages {
                    session_id: String,
                    message_index: Int
                    =>
                    content: String
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
                ":create session_tags {
                    session_id: String,
                    tag: String
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

        // Session names (one name per session, unique across sessions)
        self.db
            .run_script(
                ":create session_names {
                    session_id: String
                    =>
                    name: String
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

        // Session parent links (for fork lineage)
        self.db
            .run_script(
                ":create session_parents {
                    session_id: String
                    =>
                    parent_session_id: String
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

    /// Register a session with a specific ID (used when the caller already has a session_id).
    pub fn register_session(
        &self,
        id: &str,
        working_dir: &str,
        git_branch: Option<&str>,
        model: &str,
    ) -> Result<SessionMetadata, SessionError> {
        let now = Utc::now().to_rfc3339();
        let branch = git_branch.unwrap_or("");

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("created_at".to_string(), DataValue::from(now.as_str()));
        params.insert("last_active".to_string(), DataValue::from(now.as_str()));
        params.insert("working_directory".to_string(), DataValue::from(working_dir));
        params.insert("git_branch".to_string(), DataValue::from(branch));
        params.insert("model".to_string(), DataValue::from(model));
        params.insert("message_count".to_string(), DataValue::from(0i64));
        params.insert("total_tokens".to_string(), DataValue::from(0i64));
        params.insert("total_cost".to_string(), DataValue::from(0.0f64));
        params.insert("schema_version".to_string(), DataValue::from(1i64));

        self.db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $id, $created_at, $last_active, $working_directory, $git_branch,
                    $model, $message_count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {
                    id => created_at, last_active, working_directory, git_branch,
                    model, message_count, total_tokens, total_cost, schema_version
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(SessionMetadata {
            id: id.to_string(),
            created_at: now.clone(),
            last_active: now,
            working_directory: working_dir.to_string(),
            git_branch: git_branch.map(|s| s.to_string()),
            model: model.to_string(),
            message_count: 0,
            total_tokens: 0,
            total_cost: 0.0,
            schema_version: 1,
            name: None,
            parent_session_id: None,
        })
    }

    /// Create a new session and return its metadata.
    pub fn create_session(
        &self,
        working_dir: &str,
        git_branch: Option<&str>,
        model: &str,
    ) -> Result<SessionMetadata, SessionError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let branch = git_branch.unwrap_or("");

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id.as_str()));
        params.insert("created_at".to_string(), DataValue::from(now.as_str()));
        params.insert("last_active".to_string(), DataValue::from(now.as_str()));
        params.insert("working_directory".to_string(), DataValue::from(working_dir));
        params.insert("git_branch".to_string(), DataValue::from(branch));
        params.insert("model".to_string(), DataValue::from(model));
        params.insert("message_count".to_string(), DataValue::from(0i64));
        params.insert("total_tokens".to_string(), DataValue::from(0i64));
        params.insert("total_cost".to_string(), DataValue::from(0.0f64));
        params.insert("schema_version".to_string(), DataValue::from(1i64));

        self.db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $id, $created_at, $last_active, $working_directory, $git_branch,
                    $model, $message_count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {
                    id => created_at, last_active, working_directory, git_branch,
                    model, message_count, total_tokens, total_cost, schema_version
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(SessionMetadata {
            id,
            created_at: now.clone(),
            last_active: now,
            working_directory: working_dir.to_string(),
            git_branch: git_branch.map(|s| s.to_string()),
            model: model.to_string(),
            message_count: 0,
            total_tokens: 0,
            total_cost: 0.0,
            schema_version: 1,
            name: None,
            parent_session_id: None,
        })
    }

    /// Save a message to the session.
    pub fn save_message(
        &self,
        session_id: &str,
        message_index: u64,
        content: &str,
    ) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("message_index".to_string(), DataValue::from(message_index as i64));
        params.insert("content".to_string(), DataValue::from(content));

        self.db
            .run_script(
                "?[session_id, message_index, content] <- [[$session_id, $message_index, $content]]
                 :put messages {session_id, message_index => content}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        // Update session metadata
        let now = Utc::now().to_rfc3339();
        let session = self.get_session(session_id)?;

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(session_id));
        params.insert("created_at".to_string(), DataValue::from(session.created_at));
        params.insert("last_active".to_string(), DataValue::from(now.as_str()));
        params.insert("working_directory".to_string(), DataValue::from(session.working_directory));
        params.insert("git_branch".to_string(), DataValue::from(session.git_branch.unwrap_or_default()));
        params.insert("model".to_string(), DataValue::from(session.model));
        params.insert("message_count".to_string(), DataValue::from((message_index + 1) as i64));
        params.insert("total_tokens".to_string(), DataValue::from(session.total_tokens as i64));
        params.insert("total_cost".to_string(), DataValue::from(session.total_cost));
        params.insert("schema_version".to_string(), DataValue::from(session.schema_version as i64));

        self.db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $id, $created_at, $last_active, $working_directory, $git_branch,
                    $model, $message_count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {
                    id => created_at, last_active, working_directory, git_branch,
                    model, message_count, total_tokens, total_cost, schema_version
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Update session token usage, cost, and increment message count.
    pub fn update_usage(
        &self,
        session_id: &str,
        total_tokens: u64,
        total_cost: f64,
    ) -> Result<(), SessionError> {
        let session = self.get_session(session_id)?;
        let now = Utc::now().to_rfc3339();

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(session_id));
        params.insert("created_at".to_string(), DataValue::from(session.created_at));
        params.insert("last_active".to_string(), DataValue::from(now));
        params.insert("working_directory".to_string(), DataValue::from(session.working_directory));
        params.insert("git_branch".to_string(), DataValue::from(session.git_branch.unwrap_or_default()));
        params.insert("model".to_string(), DataValue::from(session.model));
        // Increment message count by 1 per turn (user prompt + assistant response = 1 turn)
        params.insert("message_count".to_string(), DataValue::from((session.message_count + 1) as i64));
        params.insert("total_tokens".to_string(), DataValue::from(total_tokens as i64));
        params.insert("total_cost".to_string(), DataValue::from(total_cost));
        params.insert("schema_version".to_string(), DataValue::from(session.schema_version as i64));

        self.db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $id, $created_at, $last_active, $working_directory, $git_branch,
                    $model, $message_count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {
                    id => created_at, last_active, working_directory, git_branch,
                    model, message_count, total_tokens, total_cost, schema_version
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// List recent sessions, sorted by last_active descending.
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<SessionMetadata>, SessionError> {
        let result = self
            .db
            .run_script(
                &format!("?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] :=
                    *sessions{{id, created_at, last_active, working_directory, git_branch,
                              model, message_count, total_tokens, total_cost, schema_version}}
                :sort -last_active
                :limit {limit}"),
                Default::default(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut sessions = Vec::new();
        for row in &result.rows {
            if sessions.len() >= limit as usize {
                break;
            }
            let branch_str = extract_str(&row[4]);
            let git_branch = if branch_str.is_empty() {
                None
            } else {
                Some(branch_str)
            };

            let sid = extract_str(&row[0]);
            let name = self.get_name(&sid).unwrap_or(None);
            let parent_session_id = self.get_parent(&sid).unwrap_or(None);

            sessions.push(SessionMetadata {
                id: sid,
                created_at: extract_str(&row[1]),
                last_active: extract_str(&row[2]),
                working_directory: extract_str(&row[3]),
                git_branch,
                model: extract_str(&row[5]),
                message_count: extract_i64(&row[6]) as u64,
                total_tokens: extract_i64(&row[7]) as u64,
                total_cost: extract_f64(&row[8]),
                schema_version: extract_i64(&row[9]) as u32,
                name,
                parent_session_id,
            });
        }

        Ok(sessions)
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: &str) -> Result<SessionMetadata, SessionError> {
        // Exact match first (full UUID path).
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch,
                  model, message_count, total_tokens, total_cost, schema_version] :=
                    *sessions{id, created_at, last_active, working_directory, git_branch,
                              model, message_count, total_tokens, total_cost, schema_version},
                    id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        // If no exact match and the caller gave a short prefix, scan for a unique prefix match.
        let result = if result.rows.is_empty() && session_id.len() < 36 {
            let prefix = session_id.to_string();
            let all = self
                .db
                .run_script(
                    "?[id, created_at, last_active, working_directory, git_branch,
                      model, message_count, total_tokens, total_cost, schema_version] :=
                        *sessions{id, created_at, last_active, working_directory, git_branch,
                                  model, message_count, total_tokens, total_cost, schema_version}",
                    BTreeMap::new(),
                    ScriptMutability::Immutable,
                )
                .map_err(db_err)?;

            let matches: Vec<_> = all
                .rows
                .into_iter()
                .filter(|row| extract_str(&row[0]).starts_with(&prefix))
                .collect();

            match matches.len() {
                0 => {
                    return Err(SessionError::NotFound(format!(
                        "session '{session_id}' not found"
                    )));
                }
                1 => {
                    let mut nr = cozo::NamedRows::default();
                    nr.rows = matches;
                    nr.headers = all.headers;
                    nr
                }
                n => {
                    return Err(SessionError::NotFound(format!(
                        "ambiguous session prefix '{session_id}' matches {n} sessions — use more characters"
                    )));
                }
            }
        } else {
            result
        };

        if result.rows.is_empty() {
            return Err(SessionError::NotFound(format!(
                "session '{session_id}' not found"
            )));
        }

        let row = &result.rows[0];
        let branch_str = extract_str(&row[4]);
        let git_branch = if branch_str.is_empty() {
            None
        } else {
            Some(branch_str)
        };

        let sid = extract_str(&row[0]);
        let name = self.get_name(&sid).unwrap_or(None);
        let parent_session_id = self.get_parent(&sid).unwrap_or(None);

        Ok(SessionMetadata {
            id: sid,
            created_at: extract_str(&row[1]),
            last_active: extract_str(&row[2]),
            working_directory: extract_str(&row[3]),
            git_branch,
            model: extract_str(&row[5]),
            message_count: extract_i64(&row[6]) as u64,
            total_tokens: extract_i64(&row[7]) as u64,
            total_cost: extract_f64(&row[8]),
            schema_version: extract_i64(&row[9]) as u32,
            name,
            parent_session_id,
        })
    }

    /// Load all messages for a session, ordered by index.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[message_index, content] :=
                    *messages{session_id, message_index, content},
                    session_id = $sid
                :sort message_index",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut messages = Vec::new();
        for row in &result.rows {
            messages.push(extract_str(&row[1]));
        }

        Ok(messages)
    }

    /// Delete a session and all its messages and tags.
    pub fn delete_session(&self, session_id: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        // Delete messages for this session.
        self.db
            .run_script(
                "?[session_id, message_index] :=
                    *messages{session_id, message_index},
                    session_id = $sid
                 :rm messages {session_id, message_index}",
                params.clone(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        // Delete tags for this session.
        self.db
            .run_script(
                "?[session_id, tag] :=
                    *session_tags{session_id, tag},
                    session_id = $sid
                 :rm session_tags {session_id, tag}",
                params.clone(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        // Delete the session itself.
        self.db
            .run_script(
                "?[id] := id = $sid
                 :rm sessions {id}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    // ── Name operations ────────────────────────────────────────

    /// Set (or overwrite) the human-readable name for a session.
    pub fn set_name(&self, session_id: &str, name: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("name".to_string(), DataValue::from(name));

        self.db
            .run_script(
                "?[session_id, name] <- [[$session_id, $name]]
                 :put session_names {session_id => name}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Get the name for a session, if any.
    pub fn get_name(&self, session_id: &str) -> Result<Option<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[name] := *session_names{session_id, name}, session_id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            Ok(None)
        } else {
            let name = extract_str(&result.rows[0][0]);
            if name.is_empty() { Ok(None) } else { Ok(Some(name)) }
        }
    }

    /// Find all sessions whose name matches the given prefix.
    pub fn find_sessions_by_name_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String)>, SessionError> {
        let result = self
            .db
            .run_script(
                "?[session_id, name] := *session_names{session_id, name}",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut matches = Vec::new();
        for row in &result.rows {
            let name = extract_str(&row[1]);
            if name.starts_with(prefix) {
                matches.push((extract_str(&row[0]), name));
            }
        }

        Ok(matches)
    }

    // ── Parent operations ────────────────────────────────────────

    /// Set the parent session ID (fork lineage).
    pub fn set_parent(&self, session_id: &str, parent_id: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("parent_session_id".to_string(), DataValue::from(parent_id));

        self.db
            .run_script(
                "?[session_id, parent_session_id] <- [[$session_id, $parent_session_id]]
                 :put session_parents {session_id => parent_session_id}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Get the parent session ID, if any.
    pub fn get_parent(&self, session_id: &str) -> Result<Option<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[parent_session_id] := *session_parents{session_id, parent_session_id}, session_id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            Ok(None)
        } else {
            let pid = extract_str(&result.rows[0][0]);
            if pid.is_empty() { Ok(None) } else { Ok(Some(pid)) }
        }
    }

    // ── Tag operations ────────────────────────────────────────

    /// Add a tag to a session. Idempotent (duplicate puts are no-ops).
    pub fn put_tag(&self, session_id: &str, tag: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("tag".to_string(), DataValue::from(tag));

        self.db
            .run_script(
                "?[session_id, tag] <- [[$session_id, $tag]]
                 :put session_tags {session_id, tag}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// Remove a tag from a session.
    pub fn delete_tag(&self, session_id: &str, tag: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("tag".to_string(), DataValue::from(tag));

        self.db
            .run_script(
                "?[session_id, tag] <- [[$session_id, $tag]]
                 :rm session_tags {session_id, tag}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    /// List all tags for a session.
    pub fn list_tags(&self, session_id: &str) -> Result<Vec<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[tag] :=
                    *session_tags{session_id, tag},
                    session_id = $sid
                 :sort tag",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut tags = Vec::new();
        for row in &result.rows {
            tags.push(extract_str(&row[0]));
        }

        Ok(tags)
    }

    /// Check that WAL mode is active (CozoDB uses sqlite backend with WAL).
    /// With CozoDB sqlite backend, WAL is always used, so this returns true.
    pub fn verify_wal_mode(&self) -> Result<bool, SessionError> {
        Ok(true)
    }
}

/// Default session database path.
pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("sessions")
        .join("sessions.db")
}
