//! What the tripwire keeps: pre-call objects, the violation log, the recent
//! violations overlapping calls are failed by, and the deliveries of
//! write-capable calls. Split from `input_tripwire` for size.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{ChangedInput, EnvironmentViolation};

/// Violations remembered for overlapping calls.
const RECENT: usize = 1024;

type Recent = VecDeque<(u64, PathBuf, String, ChangedInput)>;

static VIOLATIONS: Mutex<Recent> = Mutex::new(VecDeque::new());

/// Remember `call`'s changes to `project`'s inputs, found at host sequence
/// `at`.
pub fn remember_violation(at: u64, project: &Path, call: &str, changed: &[ChangedInput]) {
    let mut recent = VIOLATIONS.lock().unwrap_or_else(|e| e.into_inner());
    for change in changed {
        recent.push_back((at, project.to_path_buf(), call.to_string(), change.clone()));
    }
    while recent.len() > RECENT {
        recent.pop_front();
    }
}

/// Changes to `project`'s inputs other calls' checks found after `since` (a
/// tripwire's arm): inside that tripwire's window.
pub fn recent_violations_since(
    since: u64,
    project: &Path,
    call: &str,
) -> Vec<(String, ChangedInput)> {
    let recent = VIOLATIONS.lock().unwrap_or_else(|e| e.into_inner());
    recent
        .iter()
        .filter(|(at, root, other, _)| *at > since && root == project && other != call)
        .map(|(_, _, other, change)| (other.clone(), change.clone()))
        .collect()
}

static IN_FLIGHT: Mutex<Vec<(u64, PathBuf)>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A write call's own work, registered while it runs so other calls'
/// checks leave it alone.
pub struct InFlight(u64);

impl InFlight {
    pub fn register(own: &[PathBuf]) -> Self {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut live = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        for path in own {
            live.push((id, path.clone()));
            if let Ok(real) = path.canonicalize() {
                live.push((id, real));
            }
        }
        Self(id)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut live = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(id, _)| *id != self.0);
    }
}

/// Whether an in-flight write call owns `path`.
pub(super) fn in_flight_owns(path: &Path) -> bool {
    let live = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    live.iter().any(|(_, root)| path.starts_with(root))
}

fn delivered_path(run_root: &Path) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("project-inputs-delivered.jsonl")
}

/// Record that the write-capable `call` delivered `state` at the input `rel`.
pub(super) fn delivered(
    run_root: &Path,
    call: &str,
    rel: &str,
    state: &str,
) -> std::io::Result<()> {
    append(
        &delivered_path(run_root),
        &serde_json::json!({"call": call, "path": rel, "after": state}),
    )
}

/// Every (path, state) a write-capable call of this run delivered.
pub fn delivered_inputs(run_root: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(delivered_path(run_root)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|line| {
            Some((
                line.get("path")?.as_str()?.to_string(),
                line.get("after")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

pub(super) fn store_object(object: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = object.parent().unwrap_or(object);
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        object
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, object)
}

pub(super) fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(160)
        .collect()
}

pub(super) fn log(run_root: &Path, violation: &EnvironmentViolation) -> std::io::Result<()> {
    append(
        &run_root
            .join("write-coordination")
            .join("environment-violations.jsonl"),
        &serde_json::json!({"at": chrono::Utc::now().to_rfc3339(), "violation": violation}),
    )
}

fn append(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}
