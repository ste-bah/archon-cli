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

type Windows = Vec<(u64, PathBuf, std::sync::Arc<std::sync::atomic::AtomicBool>)>;
static WINDOWS: Mutex<Windows> = Mutex::new(Vec::new());

/// Batch G2: one armed tripwire's window over a project's inputs, and
/// whether any other window over the same project was open at any time
/// during it. A change found in a window nothing overlapped is attributed to
/// the one call or command it watched (or to an unwatched process, which a
/// re-run tells apart); in an overlapped one it is not.
#[derive(Debug)]
pub(super) struct Window {
    id: u64,
    overlapped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Window {
    pub(super) fn open(project: &Path) -> Self {
        use std::sync::atomic::Ordering;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut live = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let mut overlapped = false;
        for (_, other, flag) in live.iter() {
            if other == project {
                flag.store(true, Ordering::SeqCst);
                overlapped = true;
            }
        }
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(overlapped));
        live.push((id, project.to_path_buf(), flag.clone()));
        Self {
            id,
            overlapped: flag,
        }
    }

    /// Whether another window over the same project was open during this one.
    pub(super) fn overlapped(&self) -> bool {
        self.overlapped.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let mut live = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(id, _, _)| *id != self.id);
    }
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
            if let Ok(real) = path.canonicalize().map(archon_shell::paths::plain) {
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

/// Where every project-input state the host has seen is kept by its
/// content hash.
pub(super) fn objects_dir(run_root: &Path) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("input-tripwire")
        .join("objects")
}

/// Keep `bytes` under the run by their content hash (Batch L: what a landing
/// replaces, so a refused landing can be put back). Best effort: a state not
/// kept is one a revert reports it cannot restore.
pub fn keep_object(run_root: &Path, bytes: &[u8]) {
    let object = objects_dir(run_root).join(blake3::hash(bytes).to_hex().as_str());
    if !object.exists() {
        let _ = store_object(&object, bytes);
    }
}

/// The kept bytes whose content hash is `state`, if the run kept them.
pub fn kept_object(run_root: &Path, state: &str) -> Option<Vec<u8>> {
    if state.len() != 64 || !state.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = std::fs::read(objects_dir(run_root).join(state)).ok()?;
    (blake3::hash(&bytes).to_hex().to_string() == state).then_some(bytes)
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
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&line)?;
    file.sync_all()
}
