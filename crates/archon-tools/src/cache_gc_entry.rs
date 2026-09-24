//! One entry of the unleased store: how it is named, what it records about
//! itself, and how a user of it proves it is busy.
//!
//! Split from `cache_gc` to hold the 500-line ceiling. The sweep lives there;
//! everything an entry knows about itself lives here.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Marker file written inside every entry this code creates.
pub(super) const MARKER_FILE: &str = ".archon-cache-entry.json";
/// Stamp file recording when the store was last swept.
pub(super) const STAMP_FILE: &str = ".archon-cache-sweep";
/// Suffix of a directory that has been committed for removal.
pub(super) const TRASH_SUFFIX: &str = ".trash";

/// How the unleased store is bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheGcPolicy {
    /// Remove entries whose recorded repository path no longer exists.
    pub collect_dead_entries: bool,
    /// Ceiling on the total size of entries this code manages. Zero disables
    /// the cap.
    pub max_bytes: u64,
    /// Shortest gap between two sweeps of one store.
    pub interval: Duration,
}

impl Default for CacheGcPolicy {
    /// 64 GiB, swept at most hourly.
    ///
    /// The cap holds roughly a dozen entries at the sizes observed in practice
    /// (about 5 GiB each) — comfortably more checkouts than are open at once,
    /// so steady-state work never evicts — while staying an order of magnitude
    /// below the 258 GiB this store had reached and a small fraction of a
    /// development volume.
    ///
    /// Hourly because an entry is created only when a checkout appears that has
    /// never built here before. Sweeping more often would re-walk gigabytes of
    /// directory metadata to find nothing; sweeping less often would let a
    /// burst of short-lived branches accumulate. The guard itself is one
    /// `stat`.
    fn default() -> Self {
        Self {
            collect_dead_entries: true,
            max_bytes: 64 * 1024 * 1024 * 1024,
            interval: Duration::from_secs(3600),
        }
    }
}

static POLICY: OnceLock<RwLock<CacheGcPolicy>> = OnceLock::new();

/// Install the policy for this process.
pub fn configure(policy: CacheGcPolicy) {
    if let Ok(mut current) = POLICY
        .get_or_init(|| RwLock::new(CacheGcPolicy::default()))
        .write()
    {
        *current = policy;
    }
}

/// The policy in force, defaulting when nothing installed one.
pub fn policy() -> CacheGcPolicy {
    POLICY
        .get()
        .and_then(|p| p.read().ok().map(|p| p.clone()))
        .unwrap_or_default()
}

/// The directory name an entry for `identity` is stored under.
///
/// One-way on purpose — the path is long and full of separators — which is
/// exactly why the marker file exists.
pub fn entry_name(identity: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(identity.to_string_lossy().as_bytes())
    )
}

/// True for a name this module itself produces.
///
/// Removal is restricted to these, so a sweep pointed at the wrong directory
/// removes nothing at all.
pub(super) fn is_entry_name(name: &str) -> bool {
    name.len() == 64
        && name
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Identity and timing recorded inside an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMarker {
    /// Path of the checkout this entry caches for.
    pub repository: String,
    pub created_at: String,
    pub last_used_at: String,
}

fn marker_path(entry: &Path) -> PathBuf {
    entry.join(MARKER_FILE)
}

/// Read an entry's marker, if it has a parseable one naming a path.
pub fn read_marker(entry: &Path) -> Option<EntryMarker> {
    let text = std::fs::read_to_string(marker_path(entry)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let repository = value["repository"].as_str()?.to_string();
    if repository.is_empty() {
        return None;
    }
    Some(EntryMarker {
        repository,
        created_at: value["created_at"].as_str().unwrap_or_default().to_string(),
        last_used_at: value["last_used_at"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

/// Write or refresh the marker, preserving the original creation time.
fn write_marker(entry: &Path, repository: &Path) -> std::io::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let created_at = read_marker(entry)
        .map(|m| m.created_at)
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| now.clone());
    let marker = serde_json::json!({
        "repository": repository.to_string_lossy(),
        "created_at": created_at,
        "last_used_at": now,
    });
    std::fs::write(marker_path(entry), marker.to_string())
}

/// Path of the advisory lock for an entry.
///
/// A sibling of the entry, never a file inside it: a file within the directory
/// would be destroyed by the very `remove_dir_all` whose safety it is being
/// used to decide, and a new user could recreate the directory underneath a
/// removal already in flight.
///
/// Lock files are never deleted. They are empty, one per entry, and unlinking
/// one is the only way to reintroduce the race the sibling placement removes.
pub(super) fn lock_path(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}.lock"))
}

/// Entries this process is using, by lock path, with a use count.
///
/// The count exists because two commands in one process can share a checkout,
/// and an advisory lock is held per open file — a second handle in the same
/// process would contend with the first. The leaked `RwLock` gives the guard a
/// `'static` borrow: one small allocation per distinct entry this process
/// touches, bounded by the number of checkouts it builds in.
type HeldMap = HashMap<PathBuf, (usize, fd_lock::RwLockReadGuard<'static, File>)>;
static HELD: LazyLock<Mutex<HeldMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn held() -> MutexGuard<'static, HeldMap> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Proof that an entry is in use. The lock is released when this is dropped.
#[derive(Debug)]
pub struct CacheEntryGuard {
    lock: PathBuf,
}

impl Drop for CacheEntryGuard {
    fn drop(&mut self) {
        let mut held = held();
        if let Some((count, _)) = held.get_mut(&self.lock) {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.lock);
            }
        }
    }
}

/// Take the shared lock for an entry, or `None` if it cannot be taken.
fn hold_entry(lock: PathBuf) -> Option<CacheEntryGuard> {
    let mut held = held();
    if let Some((count, _)) = held.get_mut(&lock) {
        *count += 1;
        return Some(CacheEntryGuard { lock });
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .ok()?;
    // Leaked so the guard borrows something `'static`; see `HELD`.
    let rw: &'static fd_lock::RwLock<File> = Box::leak(Box::new(fd_lock::RwLock::new(file)));
    let guard = rw.try_read().ok()?;
    held.insert(lock.clone(), (1, guard));
    Some(CacheEntryGuard { lock })
}

/// Prepare the entry for `identity` under `root`, returning proof of use.
///
/// The marker is written before the caller builds anything, so a sweep running
/// concurrently reads a path that exists and keeps the entry. `None` for the
/// guard is not a failure: it means another process holds the lock, and that
/// protects the entry just as well.
pub fn open_entry(
    root: &Path,
    identity: &Path,
) -> std::io::Result<(PathBuf, Option<CacheEntryGuard>)> {
    std::fs::create_dir_all(root)?;
    let name = entry_name(identity);
    let guard = hold_entry(lock_path(root, &name));
    let entry = root.join(&name);
    std::fs::create_dir_all(&entry)?;
    write_marker(&entry, identity)?;
    Ok((entry, guard))
}
