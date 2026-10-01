//! One entry of the unleased store: how it is named, what it records about
//! itself, and how a user of it proves it is busy.
//!
//! Split from `cache_gc` to hold the 500-line ceiling. The sweep lives there;
//! everything an entry knows about itself lives here.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// How long an entry with no usable marker must have gone unmodified —
    /// judged by the newest mtime anywhere inside it — before a sweep may
    /// remove it, and then only while no one holds its lock. Zero means a
    /// marker-less entry is never removed.
    pub unmarked_idle: Duration,
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
    ///
    /// A day of no writes before a marker-less entry counts as idle. Every
    /// current user takes the lock and writes a marker on acquire, so this
    /// rule only ever judges entries left by builds that predate markers, or
    /// by one killed between creating the directory and writing its marker.
    /// Neither can be protected by a lock; a day without a single write inside
    /// the entry is the evidence that nothing is building there, and bounds
    /// how long an abandoned entry lingers.
    fn default() -> Self {
        Self {
            collect_dead_entries: true,
            max_bytes: 64 * 1024 * 1024 * 1024,
            interval: Duration::from_secs(3600),
            unmarked_idle: Duration::from_secs(24 * 3600),
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

/// What an entry's marker says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarkerState {
    Present(EntryMarker),
    /// No marker file at all.
    Missing,
    /// A marker exists but cannot be read or does not name a path. Carries why.
    Invalid(String),
}

/// Read an entry's marker, distinguishing absent from broken.
pub(super) fn marker_state(entry: &Path) -> MarkerState {
    let text = match std::fs::read_to_string(marker_path(entry)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return MarkerState::Missing;
        }
        Err(error) => return MarkerState::Invalid(format!("unreadable: {error}")),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => return MarkerState::Invalid(format!("not JSON: {error}")),
    };
    let repository = match value["repository"].as_str() {
        Some(repository) if !repository.is_empty() => repository.to_string(),
        _ => return MarkerState::Invalid("names no repository path".into()),
    };
    MarkerState::Present(EntryMarker {
        repository,
        created_at: value["created_at"].as_str().unwrap_or_default().to_string(),
        last_used_at: value["last_used_at"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

/// Read an entry's marker, if it has a parseable one naming a path.
pub fn read_marker(entry: &Path) -> Option<EntryMarker> {
    match marker_state(entry) {
        MarkerState::Present(marker) => Some(marker),
        MarkerState::Missing | MarkerState::Invalid(_) => None,
    }
}

/// Distinguishes concurrent marker writers' temporary files.
static MARKER_WRITES: AtomicU64 = AtomicU64::new(0);

/// Write or refresh the marker, preserving the original creation time.
///
/// Written to a temporary file and renamed into place, so a reader — a sweep,
/// or another process acquiring the same entry — sees the old marker or the new
/// one, never a truncated file that would read as broken.
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
    let tmp = entry.join(format!(
        "{MARKER_FILE}.{}.{}.tmp",
        std::process::id(),
        MARKER_WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, marker.to_string())?;
    std::fs::rename(&tmp, marker_path(entry)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
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
/// process would contend with the first.
///
/// The map owns the lock file itself. The shared lock lives exactly as long as
/// that descriptor: when the count reaches zero the entry is removed, the file
/// is closed, and closing it is what releases the lock. Nothing outlives the
/// last user, so taking and releasing an entry any number of times leaves no
/// descriptor behind.
pub(super) struct HeldEntry {
    count: usize,
    /// Holds the shared lock by being open; see `try_hold`.
    _lock: fd_lock::RwLock<File>,
}
type HeldMap = HashMap<PathBuf, HeldEntry>;
static HELD: LazyLock<Mutex<HeldMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn held() -> MutexGuard<'static, HeldMap> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Proof that an entry is in use. The lock is released when the last guard
/// for an entry in this process is dropped.
#[derive(Debug)]
pub struct CacheEntryGuard {
    lock: PathBuf,
}

impl Drop for CacheEntryGuard {
    fn drop(&mut self) {
        let mut held = held();
        if let Some(entry) = held.get_mut(&self.lock) {
            entry.count -= 1;
            if entry.count == 0 {
                // Dropping the file closes it, which releases the lock.
                held.remove(&self.lock);
            }
        }
    }
}

/// How long a user waits for a sweep that holds an entry's exclusive lock.
///
/// A sweep holds it only across the check and the atomic rename, never across
/// the removal of the renamed directory. For a marked entry the check is one
/// `stat`, so the wait is microseconds. For a marker-less one it re-walks the
/// tree to prove it still idle, which can outlast the bound; a user arriving
/// then runs that one command without the cache, never into an unlocked entry.
/// The bound exists so a wedged sweep cannot stall a command.
pub(super) const SWEEP_WAIT: Duration = Duration::from_secs(2);
const SWEEP_POLL: Duration = Duration::from_millis(20);

enum Attempt {
    Held(CacheEntryGuard),
    /// Someone holds the exclusive lock: a sweep is deciding or removing.
    Busy,
    /// The lock file cannot be opened. Not evidence of anything; do not use
    /// the entry.
    Unavailable,
}

/// One non-blocking attempt to take the shared lock, under the `HELD` mutex so
/// two threads cannot both register the same entry.
fn try_hold(lock: &Path) -> Attempt {
    let mut held = held();
    if let Some(entry) = held.get_mut(lock) {
        entry.count += 1;
        return Attempt::Held(CacheEntryGuard {
            lock: lock.to_path_buf(),
        });
    }
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)
    else {
        return Attempt::Unavailable;
    };
    let rw = fd_lock::RwLock::new(file);
    match rw.try_read() {
        // Forgotten, not dropped: dropping the guard would unlock. The lock is
        // released instead by closing the file, when `HELD` drops `rw`.
        Ok(guard) => std::mem::forget(guard),
        Err(_) => return Attempt::Busy,
    }
    held.insert(
        lock.to_path_buf(),
        HeldEntry {
            count: 1,
            _lock: rw,
        },
    );
    Attempt::Held(CacheEntryGuard {
        lock: lock.to_path_buf(),
    })
}

/// Take the shared lock for an entry, waiting up to `wait` for a sweep that
/// holds the exclusive one. `None` if it cannot be taken.
///
/// The mutex is not held while sleeping, so a slow sweep delays only the
/// command that wants this entry.
fn hold_entry(lock: &Path, wait: Duration) -> Option<CacheEntryGuard> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match try_hold(lock) {
            Attempt::Held(guard) => return Some(guard),
            Attempt::Unavailable => return None,
            Attempt::Busy if std::time::Instant::now() >= deadline => return None,
            Attempt::Busy => std::thread::sleep(SWEEP_POLL),
        }
    }
}

/// Prepare the entry for `identity` under `root`, returning its path and
/// proof of use.
///
/// The lock is taken before anything is created, and the marker written
/// before the caller builds anything, so a sweep that runs afterwards reads a
/// path that exists and a lock it cannot take.
///
/// `Ok(None)` means the entry could not be locked: a sweep held it past
/// `SWEEP_WAIT`, or the lock file could not be opened. A shared lock held by
/// another process never causes this — shared locks do not conflict. The
/// caller must not use the entry then, because nothing protects it; it runs
/// the command without the unleased cache instead.
pub fn open_entry(
    root: &Path,
    identity: &Path,
) -> std::io::Result<Option<(PathBuf, CacheEntryGuard)>> {
    open_entry_waiting(root, identity, SWEEP_WAIT)
}

pub(super) fn open_entry_waiting(
    root: &Path,
    identity: &Path,
    wait: Duration,
) -> std::io::Result<Option<(PathBuf, CacheEntryGuard)>> {
    std::fs::create_dir_all(root)?;
    let name = entry_name(identity);
    let Some(guard) = hold_entry(&lock_path(root, &name), wait) else {
        return Ok(None);
    };
    let entry = root.join(&name);
    std::fs::create_dir_all(&entry)?;
    // Every entry this code creates carries a marker before anyone builds into
    // it. If the marker cannot be written the entry is not handed out: an
    // unmarked entry in use is exactly what the sweep cannot reason about.
    write_marker(&entry, identity)?;
    Ok(Some((entry, guard)))
}
