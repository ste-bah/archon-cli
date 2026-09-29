//! The run's append-only log of every project-input decision a landing made
//! (Batch E; `project_inputs_apply`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One decision in the run's append-only project-input log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInputLanding {
    pub stage_id: String,
    pub item_id: String,
    pub task_ids: Vec<String>,
    /// Relative to the project root.
    pub path: String,
    /// `intent` (about to apply), `applied` or `refused` (a branch's
    /// change), `synced` or `sync_refused` (a tracked input a landing
    /// changed), `reverted` (Batch L: the host put back a landing a verdict
    /// of its unit refused; `before` is what it took out, `after` what it
    /// restored). For a branch's change, `before` is the baseline it was
    /// judged from.
    pub outcome: String,
    pub before: String,
    pub after: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// When it was decided, in nanoseconds since the epoch.
    pub at: i64,
}

impl ProjectInputLanding {
    pub fn refused(&self) -> bool {
        matches!(self.outcome.as_str(), "refused" | "sync_refused")
    }

    /// The project's copy now holds this line's `after`.
    pub fn landed(&self) -> bool {
        matches!(self.outcome.as_str(), "applied" | "synced")
    }

    /// The host took a refused landing of `stage_id`/`item_id` at `path`
    /// back out (Batch L); the project's copy now holds this line's `after`.
    pub fn reverted(&self) -> bool {
        self.outcome == "reverted"
    }
}

fn ledger_path(run_root: &Path) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("project-inputs.jsonl")
}

/// Every decision this run's landings made, in order. No log is an empty
/// answer. A final line with no newline is a write a crash cut short and is
/// not a decision; any other line that does not parse is an error, never
/// skipped.
pub fn run_project_input_landings(run_root: &Path) -> Result<Vec<ProjectInputLanding>, String> {
    let path = ledger_path(run_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let whole = text.rfind('\n').map_or("", |end| &text[..=end]);
    whole
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), at + 1))
        })
        .collect()
}

pub(crate) fn append(run_root: &Path, lines: &[ProjectInputLanding]) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let path = ledger_path(run_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = Vec::new();
    for line in lines {
        serde_json::to_writer(&mut bytes, line).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)?;
    // A line a crash cut short is cut away first, so every line the log
    // holds is whole and the next reader never stops at it.
    let text = std::fs::read(&path)?;
    if !text.is_empty() && !text.ends_with(b"\n") {
        let keep = text
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |at| at + 1);
        #[cfg(windows)]
        {
            // An append-only Windows handle lacks FILE_WRITE_DATA, required
            // by set_len. Repair under the caller's repository lock, retaining
            // append-only access on the handle used for new records.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_len(keep as u64)?;
        }
        #[cfg(not(windows))]
        file.set_len(keep as u64)?;
    }
    std::io::Write::write_all(&mut file, &bytes)?;
    file.sync_all()
}
