//! External ledgers for persisting decisions, tasks, and verification results.
//!
//! Each ledger writes to a JSON file under `<session_dir>/ledgers/`. Entries are
//! appended atomically (read-modify-write with fsync) and loaded back in
//! chronological (append) order.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Entry types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEntry {
    pub id: String,
    pub decision: String,
    pub reason: String,
    pub source: String,
    pub affected: Vec<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEntry {
    pub task_id: String,
    pub status: TaskStatus,
    pub assigned_agent: String,
    pub dependencies: Vec<String>,
    pub changed_files: Vec<String>,
    pub wiring_obligations: Vec<WiringObligationRef>,
    pub last_verification: Option<VerificationRef>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Blocked,
    Verified,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiringObligationRef {
    pub obligation_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRef {
    pub gate_name: String,
    pub passed: bool,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEntry {
    pub gate_name: String,
    pub passed: bool,
    pub failure_details: Option<String>,
    pub evidence_summary: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Generic I/O helpers
// ---------------------------------------------------------------------------

fn append_to_ledger<T: Serialize + for<'de> Deserialize<'de>>(path: &Path, entry: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut entries: Vec<serde_json::Value> = if path.exists() {
        let data = std::fs::read_to_string(path)?;
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    };
    entries.push(serde_json::to_value(entry)?);
    let data = serde_json::to_string_pretty(&entries)?;
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(data.as_bytes())?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn load_from_ledger<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(path)?;
    let entries: Vec<T> = serde_json::from_str(&data)?;
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Ledger structs
// ---------------------------------------------------------------------------

pub struct DecisionLedger {
    path: PathBuf,
}

impl DecisionLedger {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            path: session_dir.join("ledgers").join("decisions.json"),
        }
    }

    pub fn append(&self, entry: &DecisionEntry) -> Result<()> {
        append_to_ledger(&self.path, entry)
    }

    pub fn load_all(&self) -> Result<Vec<DecisionEntry>> {
        load_from_ledger(&self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct TaskLedger {
    path: PathBuf,
}

impl TaskLedger {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            path: session_dir.join("ledgers").join("tasks.json"),
        }
    }

    pub fn append(&self, entry: &TaskEntry) -> Result<()> {
        append_to_ledger(&self.path, entry)
    }

    pub fn load_all(&self) -> Result<Vec<TaskEntry>> {
        load_from_ledger(&self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct VerificationLedger {
    path: PathBuf,
}

impl VerificationLedger {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            path: session_dir.join("ledgers").join("verifications.json"),
        }
    }

    pub fn append(&self, entry: &VerificationEntry) -> Result<()> {
        append_to_ledger(&self.path, entry)
    }

    pub fn load_all(&self) -> Result<Vec<VerificationEntry>> {
        load_from_ledger(&self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
