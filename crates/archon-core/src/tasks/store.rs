use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

use chrono::Utc;
use crc32fast::Hasher;
use dashmap::DashMap;

use crate::tasks::models::{Task, TaskError, TaskFilter, TaskId, TaskSnapshot, TaskState};

/// Trait for task state storage and retrieval.
///
/// The hot-cache (DashMap) serves reads; persistence is added in TASK-AGS-203.
pub trait TaskStateStore: Send + Sync {
    /// Get a snapshot of a single task by ID.
    fn get_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError>;

    /// List task snapshots matching the given filter.
    fn list_snapshots(&self, filter: &TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError>;

    /// Insert or update a task snapshot in the store.
    fn put_snapshot(&self, task: &Task) -> Result<(), TaskError>;
}

/// In-memory implementation backed by DashMap (hot-cache).
///
/// Used directly by DefaultTaskService. TASK-AGS-203 adds SqliteTaskStateStore
/// for durable persistence with this same trait.
pub struct InMemoryTaskStateStore {
    snapshots: DashMap<TaskId, TaskSnapshot>,
}

impl InMemoryTaskStateStore {
    pub fn new() -> Self {
        Self {
            snapshots: DashMap::new(),
        }
    }
}

impl Default for InMemoryTaskStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStateStore for InMemoryTaskStateStore {
    fn get_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        self.snapshots
            .get(&id)
            .map(|r| r.value().clone())
            .ok_or(TaskError::NotFound(id))
    }

    fn list_snapshots(&self, filter: &TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        let mut results: Vec<TaskSnapshot> = self
            .snapshots
            .iter()
            .map(|r| r.value().clone())
            .filter(|snap| {
                if let Some(state) = &filter.state
                    && snap.state != *state
                {
                    return false;
                }
                if let Some(agent) = &filter.agent_name
                    && snap.agent_name != *agent
                {
                    return false;
                }
                if let Some(since) = &filter.since
                    && snap.created_at < *since
                {
                    return false;
                }
                true
            })
            .collect();
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    fn put_snapshot(&self, task: &Task) -> Result<(), TaskError> {
        let snapshot = TaskSnapshot::from(task);
        self.snapshots.insert(task.id, snapshot);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SqliteTaskStateStore — durable persistence (TASK-AGS-203)
// ---------------------------------------------------------------------------

/// Durable task state store backed by a SQLite index + JSON snapshot files.
///
/// Layout:
/// - `{dir}/{uuid}.json` — snapshot with CRC32 header (4-byte LE prefix)
/// - `{dir}/{uuid}/result.bin` — large results (> 64 KiB)
/// - `{dir}/index.db` — SQLite index for fast listing / rehydration
///
/// NOTE: Named "Sqlite" for future migration; currently uses the `sqlite`
/// crate (which shares `sqlite3-src` with cozo-ce, avoiding link conflicts).
pub struct SqliteTaskStateStore {
    dir: PathBuf,
    db: StdMutex<sqlite::ConnectionThreadSafe>,
    cache: DashMap<TaskId, TaskSnapshot>,
}

// sqlite::ConnectionThreadSafe is Sync but not Send.
// We only ever access it through a StdMutex, so it's safe to send the
// owning struct across threads. The Mutex ensures single-threaded access.
unsafe impl Send for SqliteTaskStateStore {}

impl SqliteTaskStateStore {
    /// Open or create the store at the given directory.
    pub fn open(dir: &Path) -> Result<Self, TaskError> {
        std::fs::create_dir_all(dir).map_err(TaskError::Io)?;

        let db_path = dir.join("index.db");
        let conn = sqlite::Connection::open_thread_safe(&db_path).map_err(sqlite_err)?;

        // Enable WAL mode for concurrent reads.
        conn.execute("PRAGMA journal_mode=WAL")
            .map_err(sqlite_err)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS task_index (
                task_id     TEXT PRIMARY KEY,
                agent_name  TEXT NOT NULL,
                state       TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                finished_at TEXT
            )",
        )
        .map_err(sqlite_err)?;

        let cache = DashMap::new();

        let store = Self {
            dir: dir.to_path_buf(),
            db: StdMutex::new(conn),
            cache,
        };

        // Hydrate cache from disk on open.
        store.hydrate_cache()?;

        Ok(store)
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn hydrate_cache(&self) -> Result<(), TaskError> {
        let ids: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut stmt = db
                .prepare("SELECT task_id FROM task_index")
                .map_err(sqlite_err)?;

            let mut collected = Vec::new();
            while let Ok(sqlite::State::Row) = stmt.next() {
                if let Ok(id_str) = stmt.read::<String, _>("task_id") {
                    collected.push(id_str);
                }
            }
            collected
        }; // db lock + stmt dropped here, before file reads.

        for id_str in ids {
            if let Ok(task_id) = id_str.parse::<TaskId>() {
                match self.read_snapshot_from_disk(task_id) {
                    Ok(snap) => {
                        self.cache.insert(task_id, snap);
                    }
                    Err(_) => {
                        // Corrupted file — insert corrupted marker.
                        let snap = TaskSnapshot {
                            id: task_id,
                            agent_name: "unknown".to_string(),
                            state: TaskState::Corrupted,
                            progress_pct: None,
                            created_at: Utc::now(),
                            started_at: None,
                            finished_at: None,
                            error: Some("snapshot file corrupted".to_string()),
                        };
                        self.cache.insert(task_id, snap);
                    }
                }
            }
        }

        Ok(())
    }

    fn read_snapshot_from_disk(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        let path = self.dir.join(format!("{}.json", id));
        let content = std::fs::read(&path).map_err(TaskError::Io)?;

        // Verify CRC32: first 4 bytes are LE checksum, rest is JSON.
        if content.len() < 4 {
            return Err(TaskError::Corrupted);
        }
        let stored_crc = u32::from_le_bytes([content[0], content[1], content[2], content[3]]);
        let json_bytes = &content[4..];

        let mut hasher = Hasher::new();
        hasher.update(json_bytes);
        let computed_crc = hasher.finalize();

        if stored_crc != computed_crc {
            return Err(TaskError::Corrupted);
        }

        serde_json::from_slice::<TaskSnapshot>(json_bytes).map_err(|_| TaskError::Corrupted)
    }

    fn write_snapshot_to_disk(&self, task: &Task) -> Result<(), TaskError> {
        let snapshot = TaskSnapshot::from(task);
        let json_bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|e| TaskError::Io(std::io::Error::other(e.to_string())))?;

        // CRC32 header.
        let mut hasher = Hasher::new();
        hasher.update(&json_bytes);
        let crc = hasher.finalize();

        let mut data = Vec::with_capacity(4 + json_bytes.len());
        data.extend_from_slice(&crc.to_le_bytes());
        data.extend_from_slice(&json_bytes);

        // Atomic write: write to .tmp then rename.
        let tmp_path = self.dir.join(format!("{}.json.tmp", task.id));
        let final_path = self.dir.join(format!("{}.json", task.id));

        std::fs::write(&tmp_path, &data).map_err(TaskError::Io)?;
        std::fs::rename(&tmp_path, &final_path).map_err(TaskError::Io)?;

        Ok(())
    }

    fn update_index(&self, task: &Task) -> Result<(), TaskError> {
        let db = self.db.lock().unwrap();

        let task_id_str = task.id.to_string();
        let state_str = format!("{:?}", task.state);
        let created_str = task.created_at.to_rfc3339();
        let finished_str = task.finished_at.map(|t| t.to_rfc3339());

        let mut stmt = db
            .prepare(
                "INSERT OR REPLACE INTO task_index \
                 (task_id, agent_name, state, created_at, finished_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .map_err(sqlite_err)?;

        stmt.bind((1, task_id_str.as_str())).map_err(sqlite_err)?;
        stmt.bind((2, task.agent_name.as_str()))
            .map_err(sqlite_err)?;
        stmt.bind((3, state_str.as_str())).map_err(sqlite_err)?;
        stmt.bind((4, created_str.as_str())).map_err(sqlite_err)?;

        match &finished_str {
            Some(s) => stmt.bind((5, s.as_str())).map_err(sqlite_err)?,
            None => stmt.bind((5, sqlite::Value::Null)).map_err(sqlite_err)?,
        };

        // Execute the statement (consume all rows/steps).
        stmt.next().map_err(sqlite_err)?;

        Ok(())
    }
}

/// Helper to convert a `sqlite::Error` into a `TaskError::Io`.
fn sqlite_err(e: sqlite::Error) -> TaskError {
    TaskError::Io(std::io::Error::other(e.to_string()))
}

impl TaskStateStore for SqliteTaskStateStore {
    fn get_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        self.cache
            .get(&id)
            .map(|r| r.value().clone())
            .ok_or(TaskError::NotFound(id))
    }

    fn list_snapshots(&self, filter: &TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        let mut results: Vec<TaskSnapshot> = self
            .cache
            .iter()
            .map(|r| r.value().clone())
            .filter(|snap| {
                if let Some(state) = &filter.state
                    && snap.state != *state
                {
                    return false;
                }
                if let Some(agent) = &filter.agent_name
                    && snap.agent_name != *agent
                {
                    return false;
                }
                if let Some(since) = &filter.since
                    && snap.created_at < *since
                {
                    return false;
                }
                true
            })
            .collect();
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    fn put_snapshot(&self, task: &Task) -> Result<(), TaskError> {
        // 1. Write JSON snapshot to disk (atomic).
        self.write_snapshot_to_disk(task)?;

        // 2. Update SQLite index.
        self.update_index(task)?;

        // 3. Update hot cache.
        let snapshot = TaskSnapshot::from(task);
        self.cache.insert(task.id, snapshot);

        Ok(())
    }
}
