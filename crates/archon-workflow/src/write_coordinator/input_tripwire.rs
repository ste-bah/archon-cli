//! Batch G: the host's tripwire over the project's acceptance inputs.
//!
//! A READ-ONLY verifier ran the product's own regenerator against the live
//! project root from its shell and rewrote a tracked project input. The next
//! acceptance round's scratch refused the diverged copy, and that one site
//! failure reached the implementing tasks as eleven "fix the implementation"
//! findings. The shell boundary (`bash_write_sandbox`) now bounds every agent
//! call where the platform can; this is the defence in depth behind it, and
//! the only guard over what a host-run command (an acceptance check, a floor,
//! a baseline test) or an unbounded process does to those inputs.
//!
//! [`InputTripwire::arm`] records the state of every file under the run's
//! acceptance policy `project_inputs` (the set acceptance scratch copies and
//! write branches are seeded with, tracked copies included) and keeps each
//! distinct content once under the run, where no agent may write.
//! [`InputTripwire::check`] compares after the call. A change the host
//! itself made meanwhile (a landing, a sync, a restore — every one goes
//! through [`note_host_write`]) is expected; any other is a violation: the
//! changed copy is kept as a backup, the pre-call copy is put back, and the
//! caller fails the call as an environment violation naming it.
//!
//! A restore never undoes a host write: a file the host wrote during the
//! call and something else changed after is left as it is and named, for a
//! person. Every call whose window holds the change is told of it (the
//! culprit, and any sibling armed before the change was found), so the
//! culprit never passes on a restored file; but only a change this call's
//! own check found, in a window no other watched call over the project
//! overlapped, is `attributed` to it (Batch G2). An unattributed violation
//! is the host's to resolve -- re-run the call, or report an operational
//! error -- and is never charged to a task. A path an in-flight write call
//! owns (its worktree) is left to that call's own tripwire.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::project_inputs::{ProjectInputPolicy, read_no_follow, refuse_links, write_file};

/// Most files one arm records; a tree past it is recorded as truncated.
const MAX_FILES: usize = 200_000;
/// Host writes remembered; an arm older than the oldest is never excused.
const MAX_HOST_WRITES: usize = 65_536;

#[derive(Default)]
struct HostWrites {
    seq: u64,
    /// Oldest sequence number still held.
    floor: u64,
    entries: VecDeque<(u64, PathBuf, String)>,
}

static HOST_WRITES: Mutex<HostWrites> = Mutex::new(HostWrites {
    seq: 0,
    floor: 0,
    entries: VecDeque::new(),
});
/// Held by every host landing, arm and check: a check never sees a landing
/// half made, and never restores over one.
static SECTION: Mutex<()> = Mutex::new(());

/// The host's write section: held for the whole of a landing, so no
/// tripwire arms or checks while the host is writing the project.
pub fn host_write_section() -> std::sync::MutexGuard<'static, ()> {
    SECTION.lock().unwrap_or_else(|e| e.into_inner())
}

/// `path` as it is compared: its parent canonical, when it exists.
fn key(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(archon_shell::paths::plain)
            .map_or_else(|_| path.to_path_buf(), |parent| parent.join(name)),
        _ => path.to_path_buf(),
    }
}

/// Record that the HOST put `state` (a blake3 hex digest, or `absent`) at
/// `path` in the project root. Called by every host write of a project input.
pub fn note_host_write(path: &Path, state: &str) {
    let path = key(path);
    let mut writes = HOST_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    writes.seq += 1;
    let seq = writes.seq;
    writes.entries.push_back((seq, path, state.into()));
    while writes.entries.len() > MAX_HOST_WRITES {
        if let Some((dropped, _, _)) = writes.entries.pop_front() {
            writes.floor = dropped;
        }
    }
}

/// A fresh point on the host-write sequence: an arm or a detection. Every
/// host write before it has a smaller number, every one after a larger.
fn host_sequence() -> u64 {
    let mut writes = HOST_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    writes.seq += 1;
    writes.seq
}

/// The states the host wrote at `path` after `since`, oldest first, and
/// whether the record reaches back that far.
fn host_writes_since(path: &Path, since: u64) -> (Vec<String>, bool) {
    let path = key(path);
    let writes = HOST_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let states = writes
        .entries
        .iter()
        .filter(|(seq, at, _)| *seq > since && *at == path)
        .map(|(_, _, state)| state.clone())
        .collect();
    (states, writes.floor <= since)
}

/// A file's recorded state: its blake3 digest, or a marker.
fn state_of(path: &Path) -> (String, Option<Vec<u8>>) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("absent".into(), None),
        Ok(meta) if meta.file_type().is_symlink() => (
            format!(
                "link:{}",
                std::fs::read_link(path)
                    .map(|target| target.display().to_string())
                    .unwrap_or_default()
            ),
            None,
        ),
        Ok(meta) if meta.is_file() => match read_no_follow(path) {
            Ok(bytes) => (blake3::hash(&bytes).to_hex().to_string(), Some(bytes)),
            Err(_) => ("<unreadable>".into(), None),
        },
        Ok(_) => ("<not a file>".into(), None),
        Err(_) => ("<unreadable>".into(), None),
    }
}

/// Every non-directory path under the policy's inputs, relative, sorted.
fn walk(policy: &ProjectInputPolicy) -> (Vec<String>, bool) {
    fn visit(policy: &ProjectInputPolicy, rel: &Path, out: &mut Vec<String>) -> bool {
        if policy.excluded(rel) || not_an_input(policy, rel) {
            return true;
        }
        if out.len() >= MAX_FILES {
            return false;
        }
        let path = policy.project.join(rel);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {
                let Ok(entries) = std::fs::read_dir(&path) else {
                    out.push(
                        rel.to_string_lossy()
                            .replace(std::path::MAIN_SEPARATOR, "/"),
                    );
                    return true;
                };
                let mut names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
                names.sort();
                names
                    .into_iter()
                    .all(|name| visit(policy, &rel.join(name), out))
            }
            Ok(_) => {
                out.push(
                    rel.to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/"),
                );
                true
            }
            Err(_) => true,
        }
    }
    let mut out = Vec::new();
    let mut complete = true;
    for input in &policy.inputs {
        complete &= visit(policy, input, &mut out);
    }
    // Issue-226: every allowlisted external data directory, by absolute
    // path (the project root joined with one is that path).
    for tree in policy.external.allowed() {
        complete &= visit(policy, tree, &mut out);
    }
    out.sort();
    out.dedup();
    (out, complete)
}

/// Paths under an input the tripwire never judges: the task set and the
/// engine's own `.archon` files and namespaces (the host writes those), and
/// toolchain caches (`__pycache__` beside a script the check ran).
fn not_an_input(policy: &ProjectInputPolicy, rel: &Path) -> bool {
    let tasks = policy.task_root.strip_prefix(&policy.project).ok();
    if tasks.is_some_and(|tasks| !tasks.as_os_str().is_empty() && rel.starts_with(tasks)) {
        return true;
    }
    let parts: Vec<&str> = rel.iter().filter_map(|part| part.to_str()).collect();
    let engine = parts.first() == Some(&".archon")
        && (parts.len() == 2 && policy.project.join(rel).is_file()
            || parts.get(1).is_some_and(|namespace| {
                crate::write_coordinator::patch_apply::ENGINE_LOADED.contains(namespace)
            }));
    // Only unambiguous cache names: `target` or `venv` may be data.
    let cache = |part: &&str| {
        (part.starts_with('.') || matches!(*part, "__pycache__" | "node_modules"))
            && crate::v2::write::SHARED_TOOLCHAIN_DIRS.contains(part)
    };
    engine || parts.iter().any(cache)
}

/// The project's inputs as they stood before one call.
#[derive(Debug)]
pub struct InputTripwire {
    run_root: PathBuf,
    policy: ProjectInputPolicy,
    armed_at: u64,
    files: BTreeMap<String, String>,
    complete: bool,
    /// A write-capable call's own working tree (a serial or coordinated
    /// write works in the canonical checkout, which may hold the inputs):
    /// its writes there are its work, landed by the host's commit.
    exempt: Vec<PathBuf>,
    window: records::Window,
}

/// One input a call changed, and what the host did about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedInput {
    pub path: String,
    pub before: String,
    pub after: String,
    pub restored: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A call that changed the project's inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentViolation {
    pub call: String,
    pub changed: Vec<ChangedInput>,
    /// Where each changed copy was kept before the restore.
    pub backup_dir: PathBuf,
    /// Batch G2: this call's own check found every change, and no other
    /// watched call or command over the project overlapped its window. Only
    /// then may the change be pinned on it; otherwise it is the host's to
    /// resolve (re-run, restore, or an operational error), never a task's.
    pub attributed: bool,
}

impl EnvironmentViolation {
    /// One line for a result summary, an error or a log.
    pub fn message(&self) -> String {
        let files: Vec<String> = self
            .changed
            .iter()
            .map(|change| {
                format!(
                    "{} ({} -> {}; {})",
                    change.path,
                    short(&change.before),
                    short(&change.after),
                    if change.restored {
                        "restored by the host".to_string()
                    } else {
                        format!("NOT restored: {}", change.note)
                    }
                )
            })
            .collect();
        format!(
            "ENVIRONMENT VIOLATION: {} changed the project's acceptance inputs, which only the host's landings may write: {}. The changed copies are kept under {}. The call's result is not trusted; re-run it.",
            self.call,
            files.join(", "),
            self.backup_dir.display()
        )
    }

    /// Whether every change was put back.
    pub fn restored(&self) -> bool {
        self.changed.iter().all(|change| change.restored)
    }
}

fn short(state: &str) -> &str {
    if state.len() == 64 && state.bytes().all(|b| b.is_ascii_hexdigit()) {
        &state[..12]
    } else {
        state
    }
}

impl InputTripwire {
    /// Record the run's project inputs now. `None` when the run recorded no
    /// acceptance policy with project inputs: there is nothing to watch.
    pub fn arm(run_root: &Path) -> Option<Self> {
        let policy = ProjectInputPolicy::recorded(run_root)
            .filter(|policy| !policy.inputs.is_empty() || !policy.external.is_empty())?;
        let _section = host_write_section();
        let armed_at = host_sequence();
        let window = records::Window::open(&policy.project);
        let (paths, complete) = walk(&policy);
        let objects = objects_dir(run_root);
        let mut kept = 0u64;
        let mut files = BTreeMap::new();
        for rel in paths {
            let (state, bytes) = state_of(&policy.project.join(&rel));
            if let Some(bytes) = bytes {
                let object = objects.join(&state);
                if !object.exists() && kept + bytes.len() as u64 <= policy.limit {
                    kept += bytes.len() as u64;
                    let _ = store_object(&object, &bytes);
                }
            }
            files.insert(rel, state);
        }
        Some(Self {
            run_root: run_root.to_path_buf(),
            policy,
            armed_at,
            files,
            complete,
            exempt: Vec::new(),
            window,
        })
    }

    /// Leave changes under `root` (a write-capable call's working tree, under
    /// every spelling) to the call: they are its work.
    #[must_use]
    pub fn exempting(mut self, root: &Path) -> Self {
        self.exempt.push(root.to_path_buf());
        if let Ok(real) = root.canonicalize().map(archon_shell::paths::plain) {
            self.exempt.push(real);
        }
        self
    }
}

/// Remove a project input as the host, recording that it did.
pub fn remove_input(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)?;
    note_host_write(path, "absent");
    Ok(())
}

#[path = "input_tripwire_records.rs"]
mod records;
pub use records::{
    InFlight, delivered_inputs, keep_object, kept_object, recent_violations_since,
    remember_violation,
};
use records::{delivered, in_flight_owns, log, objects_dir, sanitize, store_object};
#[path = "input_tripwire_check.rs"]
mod check;
#[path = "input_tripwire_pending.rs"]
mod pending;
#[path = "input_tripwire_scope.rs"]
mod scope;
pub use scope::{
    LandingSection, landing_section, reconcile_owned, watch, watch_exempting, watch_owned,
    watch_sync,
};

#[cfg(test)]
#[path = "input_tripwire_tests.rs"]
mod tests;
