//! CronTask struct and JSON persistence for TASK-CLI-311.
//!
//! CronTask matches the reference `CronTask` type from project-zero's
//! `agentSdkTypes.ts:283`. The `name` field is NOT part of CronTask — it is
//! stored in `archon_metadata` in the JSON file and never in the task struct.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CronTask (matches reference type)
// ---------------------------------------------------------------------------

/// Scheduled cron task.
///
/// Matches project-zero's `CronTask` type:
/// - `id` — UUID string
/// - `cron` — 5-field cron expression
/// - `prompt` — agent prompt to run when fired
/// - `created_at` — epoch milliseconds
/// - `recurring` — `None`/absent = recurring (default); `Some(false)` = one-shot
///
/// The `name` field is intentionally absent — it is stored in `archon_metadata`
/// in the JSON file, keyed by task ID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring: Option<bool>,
}

// ---------------------------------------------------------------------------
// CronFile — on-disk representation
// ---------------------------------------------------------------------------

/// Full content of `scheduled_tasks.json`.
///
/// Top-level keys:
/// - `tasks` — ordered list of `CronTask` objects (the canonical list)
/// - `archon_metadata` — map from task ID to Archon-only metadata (e.g. `name`)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CronFile {
    #[serde(default)]
    tasks: Vec<CronTask>,
    #[serde(default)]
    archon_metadata: std::collections::HashMap<String, TaskMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TaskMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// CronStore — JSON file I/O
// ---------------------------------------------------------------------------

/// Handle for reading and writing `.claude/scheduled_tasks.json`.
pub struct CronStore {
    path: PathBuf,
}

impl CronStore {
    /// Create a new store pointing at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path to the JSON file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load tasks from disk.  Returns empty list if the file does not exist.
    pub fn load(&self) -> anyhow::Result<Vec<CronTask>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        let file: CronFile = serde_json::from_str(&raw)?;
        Ok(file.tasks)
    }

    /// Save the given task list to disk (overwrites all tasks; preserves metadata).
    pub fn save(&self, tasks: &[CronTask]) -> anyhow::Result<()> {
        // Preserve existing metadata
        let existing = self.load_file().unwrap_or_default();
        let file = CronFile {
            tasks: tasks.to_vec(),
            archon_metadata: existing.archon_metadata,
        };
        self.write_file(&file)
    }

    /// Append a task without a name to the store.
    pub fn add(&self, task: CronTask) -> anyhow::Result<()> {
        self.add_with_name(task, None)
    }

    /// Append a task with an optional name to the store.
    ///
    /// The name is stored in `archon_metadata[task.id].name`, not in the task struct.
    pub fn add_with_name(&self, task: CronTask, name: Option<&str>) -> anyhow::Result<()> {
        self.ensure_dir()?;
        let mut file = self.load_file().unwrap_or_default();
        let id = task.id.clone();
        file.tasks.push(task);
        if let Some(n) = name {
            file.archon_metadata.entry(id).or_default().name = Some(n.to_string());
        }
        self.write_file(&file)
    }

    /// Remove the task with `id`.  No-op if `id` does not exist.
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut file = self.load_file()?;
        file.tasks.retain(|t| t.id != id);
        file.archon_metadata.remove(id);
        self.write_file(&file)
    }

    /// Remove task by ID, returning error if not found.
    pub fn delete_required(&self, id: &str) -> anyhow::Result<()> {
        if !self.path.exists() {
            anyhow::bail!("CronDelete: task '{id}' not found (no tasks file)");
        }
        let mut file = self.load_file()?;
        let before = file.tasks.len();
        file.tasks.retain(|t| t.id != id);
        if file.tasks.len() == before {
            anyhow::bail!("CronDelete: task '{id}' not found");
        }
        file.archon_metadata.remove(id);
        self.write_file(&file)
    }

    /// Return the optional name for a task.
    pub fn get_name(&self, id: &str) -> Option<String> {
        self.load_file()
            .ok()
            .and_then(|f| f.archon_metadata.get(id)?.name.clone())
    }

    // ── Private helpers ─────────────────────────────────────────────────────

    fn load_file(&self) -> anyhow::Result<CronFile> {
        if !self.path.exists() {
            return Ok(CronFile::default());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    fn write_file(&self, file: &CronFile) -> anyhow::Result<()> {
        self.ensure_dir()?;
        let json = serde_json::to_string_pretty(file)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    fn ensure_dir(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}
