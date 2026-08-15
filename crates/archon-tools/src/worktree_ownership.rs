//! Who owns a worktree directory, and is that owner still alive?
//!
//! Worktrees used to be named after the **session**, and every agent in a
//! session shares one session id — so every agent resolved to one directory and
//! `create_worktree` deleted whatever it found there. Three ways that lost
//! work: two agents in a session colliding, a resumed agent wiping its own
//! previous run, and a subagent's exit lookup resolving its *parent's* tree.
//!
//! The fix is in two halves. This module owns both.
//!
//! **Naming.** A directory is keyed by the agent that owns it, never by the
//! session. Collisions stop being a race and become impossible.
//!
//! **Liveness.** Before anything is deleted, the owner must be shown to be
//! gone. An in-process registry cannot show that on its own:
//!
//! - cancellation marks an agent terminal immediately while the runner is
//!   still given up to 30s to drain, so a "dead" agent may still be writing;
//! - finished entries are reaped ~60s later leaving no tombstone, so after a
//!   restart every directory looks abandoned;
//! - `worktrees/` is user-global — a second archon process shares it and is
//!   invisible to the first.
//!
//! So liveness is asked of the filesystem instead, with an advisory lock held
//! for the worktree's lifetime: *can I take the lock?* That is immune to PID
//! reuse (which matters on Windows), releases itself if the process dies, and
//! is cross-process by construction. `archon-cozo` uses the same crate for the
//! same reason. The marker file beside it carries identity only, so a refusal
//! can name the owner rather than just saying no.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

/// Worktrees this process currently owns, by owner key.
///
/// The value is the live write guard: dropping it releases the OS lock, so
/// removing the entry *is* the release. The `RwLock` it borrows is leaked on
/// purpose — one small allocation per worktree this process creates, bounded by
/// the number of concurrent agents, in exchange for a `'static` guard that can
/// outlive the call that took it. A worktree's lock has to survive from
/// creation until exit, which is many calls apart.
static HELD_LOCKS: LazyLock<Mutex<HashMap<String, fd_lock::RwLockWriteGuard<'static, File>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn held_locks() -> MutexGuard<'static, HashMap<String, fd_lock::RwLockWriteGuard<'static, File>>> {
    // A poisoned map means some other thread panicked mid-registration, not
    // that the entries are wrong. Recovering keeps a panic in one agent from
    // making every later worktree unopenable.
    HELD_LOCKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a directory's owner is doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLiveness {
    /// This process owns it. Re-entry is reuse, not a collision.
    Ours,
    /// Someone else holds the lock — another agent, or another archon.
    /// Deleting would destroy live work.
    Foreign,
    /// Nobody holds the lock. Safe to take over.
    Free,
}

/// Identity recorded beside a worktree so a refusal can name its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerIdentity {
    pub owner_id: String,
    pub session_id: String,
    pub created_at: String,
}

/// Owner key for the top-level agent of a session.
///
/// Prefixed rather than bare so it can never collide with a subagent key, and
/// so an operator reading `worktrees/` can tell the two apart.
pub fn session_owner_key(session_id: &str) -> String {
    format!("session-{}", sanitize_key(session_id))
}

/// Owner key for a subagent.
///
/// Matches the `subagent-{id}` convention the isolation path already writes,
/// which was duplicated at three call sites and is now stated once.
pub fn subagent_owner_key(subagent_id: &str) -> String {
    format!("subagent-{}", sanitize_key(subagent_id))
}

/// The owner key for a tool invocation.
///
/// `subagent_id` is `None` only for the top-level agent — that is a real
/// answer, not missing data, so it selects the session key rather than
/// falling back to one.
pub fn owner_key_for(session_id: &str, subagent_id: Option<&str>) -> String {
    match subagent_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => subagent_owner_key(id),
        None => session_owner_key(session_id),
    }
}

/// Path of the advisory lock for `owner_id`.
///
/// Deliberately a sibling of the worktree directory, not a file inside it:
/// `remove_dir_all` on a reclaim would otherwise delete the very file whose
/// lock is being used to decide whether the reclaim is safe.
pub fn lock_path(worktrees_dir: &Path, owner_id: &str) -> PathBuf {
    worktrees_dir.join(format!("{owner_id}.owner-lock"))
}

/// Path of the identity marker for `owner_id`.
pub fn marker_path(worktrees_dir: &Path, owner_id: &str) -> PathBuf {
    worktrees_dir.join(format!("{owner_id}.owner.json"))
}

/// Ask the filesystem whether `owner_id` is still held.
///
/// Checks this process first: `fd-lock` reports a lock we already hold as
/// unavailable, which is indistinguishable from a foreign holder and would
/// make an agent refuse to re-enter its own worktree.
pub fn owner_liveness(worktrees_dir: &Path, owner_id: &str) -> OwnerLiveness {
    if held_locks().contains_key(owner_id) {
        return OwnerLiveness::Ours;
    }

    let path = lock_path(worktrees_dir, owner_id);
    if !path.exists() {
        return OwnerLiveness::Free;
    }

    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
        // Unopenable is not evidence of absence. Treat it as held: refusing to
        // delete costs an error message, deleting a live tree costs work.
        return OwnerLiveness::Foreign;
    };

    let mut lock = fd_lock::RwLock::new(file);
    match lock.try_write() {
        Ok(guard) => {
            drop(guard);
            OwnerLiveness::Free
        }
        Err(_) => OwnerLiveness::Foreign,
    }
}

/// Take the lock for `owner_id` and hold it until [`release`].
///
/// Returns `Ok(false)` when this process already holds it — re-entry is not an
/// error, it is a resume. `Err` means someone else owns it.
pub fn acquire(worktrees_dir: &Path, owner_id: &str) -> Result<bool, String> {
    if held_locks().contains_key(owner_id) {
        return Ok(false);
    }

    std::fs::create_dir_all(worktrees_dir)
        .map_err(|e| format!("Failed to create worktrees directory: {e}"))?;

    let path = lock_path(worktrees_dir, owner_id);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("Failed to open worktree lock {}: {e}", path.display()))?;

    // Leaked so the guard borrows something `'static`; see `HELD_LOCKS`.
    let lock: &'static mut fd_lock::RwLock<File> = Box::leak(Box::new(fd_lock::RwLock::new(file)));
    match lock.try_write() {
        Ok(guard) => {
            held_locks().insert(owner_id.to_string(), guard);
            Ok(true)
        }
        Err(_) => Err(format!(
            "worktree '{owner_id}' is locked by another process"
        )),
    }
}

/// Release this process's lock on `owner_id`, if it holds one.
pub fn release(owner_id: &str) {
    held_locks().remove(owner_id);
}

/// Write the identity marker.
pub fn write_marker(
    worktrees_dir: &Path,
    owner_id: &str,
    session_id: &str,
    created_at: &str,
) -> Result<(), String> {
    let marker = serde_json::json!({
        "owner_id": owner_id,
        "session_id": session_id,
        "created_at": created_at,
    });
    std::fs::write(marker_path(worktrees_dir, owner_id), marker.to_string())
        .map_err(|e| format!("Failed to write worktree owner marker: {e}"))
}

/// Read the identity marker, if one is present and parseable.
pub fn read_marker(worktrees_dir: &Path, owner_id: &str) -> Option<OwnerIdentity> {
    let text = std::fs::read_to_string(marker_path(worktrees_dir, owner_id)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(OwnerIdentity {
        owner_id: value["owner_id"].as_str().unwrap_or(owner_id).to_string(),
        session_id: value["session_id"].as_str().unwrap_or_default().to_string(),
        created_at: value["created_at"].as_str().unwrap_or_default().to_string(),
    })
}

/// Remove the lock and marker files for `owner_id`.
///
/// Called when a worktree is genuinely gone. Releases first, so the lock file
/// is not still held open when it is unlinked.
pub fn forget(worktrees_dir: &Path, owner_id: &str) {
    release(owner_id);
    let _ = std::fs::remove_file(marker_path(worktrees_dir, owner_id));
    let _ = std::fs::remove_file(lock_path(worktrees_dir, owner_id));
}

/// Describe the owner of `owner_id` for an error message.
pub fn describe_owner(worktrees_dir: &Path, owner_id: &str) -> String {
    match read_marker(worktrees_dir, owner_id) {
        Some(identity) if !identity.session_id.is_empty() => format!(
            "agent '{}' (session '{}', since {})",
            identity.owner_id, identity.session_id, identity.created_at
        ),
        Some(identity) => format!("agent '{}'", identity.owner_id),
        None => format!("agent '{owner_id}'"),
    }
}

/// Keep an owner key usable as a single path component.
fn sanitize_key(raw: &str) -> String {
    let mapped: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.chars().take(96).collect()
    }
}

#[cfg(test)]
#[path = "worktree_ownership_tests.rs"]
mod tests;
