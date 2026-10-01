//! What one sweep removes and keeps, judged by the store's directory listing
//! afterwards — the source of truth — rather than by what `sweep` returns.
//! Every test runs against a fresh temporary directory, with real files, real
//! mtimes and real advisory locks.

use super::super::cache_gc_entry::MARKER_FILE;
use super::super::cache_gc_scan::unmarked_removable;
use super::super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::os::unix::fs::PermissionsExt;

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(24 * 3600);

fn policy(collect_dead_entries: bool, max_bytes: u64) -> CacheGcPolicy {
    CacheGcPolicy {
        collect_dead_entries,
        max_bytes,
        interval: Duration::from_secs(0),
        unmarked_idle: HOUR,
    }
}

/// A synthetic store: entry directories under `root`, remembered by label so
/// listings read as words rather than hashes.
struct Store {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    root: PathBuf,
    labels: BTreeMap<String, String>,
}

impl Store {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let root = base.join("unleased");
        std::fs::create_dir_all(&root).unwrap();
        Self {
            _tmp: tmp,
            base,
            root,
            labels: BTreeMap::new(),
        }
    }

    /// An entry holding `bytes` in a nested build tree, with `marker` written
    /// verbatim when given. Returns its directory.
    fn entry(&mut self, label: &str, bytes: usize, marker: Option<String>) -> PathBuf {
        let name = entry_name(&self.base.join("checkouts").join(label));
        let dir = self.root.join(&name);
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("target/debug/payload"), vec![b'x'; bytes]).unwrap();
        if let Some(marker) = marker {
            std::fs::write(dir.join(MARKER_FILE), marker).unwrap();
        }
        self.labels.insert(name, label.to_string());
        dir
    }

    fn name(&self, label: &str) -> String {
        self.labels
            .iter()
            .find(|(_, l)| l.as_str() == label)
            .map(|(n, _)| n.clone())
            .unwrap()
    }

    /// The entry directories present, by label, printed with their `du`.
    fn listing(&self, when: &str) -> BTreeSet<String> {
        let mut present = BTreeSet::new();
        let mut total = 0;
        eprintln!("--- {when}: {}", self.root.display());
        let mut names: Vec<_> = std::fs::read_dir(&self.root)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().to_string();
            if !e.file_type().unwrap().is_dir() {
                eprintln!("    file  {name}");
                continue;
            }
            let label = self.labels.get(&name).cloned().unwrap_or(name.clone());
            let bytes = du(&e.path());
            total += bytes;
            eprintln!("    dir   {:<30} {}..  {bytes} B", label, &name[..12]);
            present.insert(label);
        }
        eprintln!("    du of entries: {total} B");
        present
    }
}

/// Bytes in regular files below `path`. Deliberately independent of the code
/// under test.
fn du(path: &Path) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(path).unwrap() {
        let e = e.unwrap();
        let meta = std::fs::symlink_metadata(e.path()).unwrap();
        if meta.is_dir() {
            total += du(&e.path());
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

/// Backdate `path` and everything below it by `ago`. Setting a child's times
/// leaves its parent's mtime alone, so the order does not matter.
fn age(path: &Path, ago: Duration) {
    let when = SystemTime::now() - ago;
    let mut pending = vec![path.to_path_buf()];
    while let Some(p) = pending.pop() {
        if p.is_dir() {
            pending.extend(std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()));
        }
        std::fs::File::open(&p).unwrap().set_modified(when).unwrap();
    }
}

fn marker(checkout: &Path, last_used: SystemTime) -> String {
    let stamp = chrono::DateTime::<chrono::Utc>::from(last_used).to_rfc3339();
    serde_json::json!({
        "repository": checkout.to_string_lossy(),
        "created_at": stamp,
        "last_used_at": stamp,
    })
    .to_string()
}

/// A shared lock from a descriptor this module does not know about — what a
/// second archon process holds while it builds.
fn foreign_shared_lock(root: &Path, name: &str) -> fd_lock::RwLock<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(format!("{name}.lock")))
        .unwrap();
    fd_lock::RwLock::new(file)
}

fn set<const N: usize>(labels: [&str; N]) -> BTreeSet<String> {
    labels.iter().map(|l| l.to_string()).collect()
}

fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn one_pass_removes_exactly_the_dead_and_the_idle_unlocked_marker_less() {
    let mut store = Store::new();
    let live = store.base.join("live-checkout");
    std::fs::create_dir_all(&live).unwrap();
    let gone = store.base.join("gone-checkout");
    let two_days_ago = SystemTime::now() - 2 * DAY;

    let e = store.entry("marked-idle", 10_000, Some(marker(&live, two_days_ago)));
    age(&e, 2 * DAY);
    let e = store.entry("marked-dead", 10_000, Some(marker(&gone, two_days_ago)));
    age(&e, 2 * DAY);
    let e = store.entry("unmarked-idle-unlocked", 10_000, None);
    age(&e, 2 * DAY);
    let e = store.entry("unmarked-idle-locked", 10_000, None);
    age(&e, 2 * DAY);
    store.entry("unmarked-fresh", 10_000, None);
    let lock = foreign_shared_lock(&store.root, &store.name("unmarked-idle-locked"));
    let held = lock.try_read().unwrap();

    let before = store.listing("before");
    let report = sweep(&store.root, &policy(true, 0));
    let after = store.listing("after");

    assert_eq!(before.len(), 5);
    assert_eq!(
        after,
        set(["marked-idle", "unmarked-fresh", "unmarked-idle-locked"])
    );
    assert_eq!(report.in_use, vec![store.name("unmarked-idle-locked")]);
    drop(held);
}

#[test]
fn the_cap_counts_marker_less_entries_and_evicts_least_recently_used_first() {
    let mut store = Store::new();
    let live = store.base.join("live-checkout");
    std::fs::create_dir_all(&live).unwrap();
    let now = SystemTime::now();

    let e = store.entry("unmarked-idle-10d", 10_000, None);
    age(&e, 10 * DAY);
    let e = store.entry("marked-used-5d", 10_000, Some(marker(&live, now - 5 * DAY)));
    age(&e, 5 * DAY);
    let e = store.entry("marked-used-1d", 10_000, Some(marker(&live, now - DAY)));
    age(&e, DAY);
    store.entry("unmarked-fresh", 10_000, None);

    // Collection off, so only the cap acts. Room for two of the four.
    store.listing("before");
    sweep(&store.root, &policy(false, 25_000));
    let after_cap = store.listing("after cap 25000");
    assert_eq!(after_cap, set(["marked-used-1d", "unmarked-fresh"]));

    // A cap no entry fits under: the marked one goes, the fresh marker-less
    // one is never evicted however far over the cap the store is.
    sweep(&store.root, &policy(false, 1));
    let after_tiny = store.listing("after cap 1");
    assert_eq!(after_tiny, set(["unmarked-fresh"]));
}

#[test]
fn an_empty_or_missing_store_sweeps_cleanly() {
    let store = Store::new();

    let report = sweep(&store.root, &policy(true, 1));

    assert!(store.listing("after").is_empty());
    assert_eq!(report.total_bytes, 0);
    let missing = store.base.join("never-created");
    sweep(&missing, &policy(true, 1));
    assert!(!missing.exists(), "a sweep must not create the store");
}

#[test]
fn a_broken_marker_is_reported_and_treated_as_marker_less() {
    let mut store = Store::new();
    let e = store.entry("garbage-idle", 100, Some("{not json".into()));
    age(&e, 2 * DAY);
    let e = store.entry("no-repository-idle", 100, Some("{}".into()));
    age(&e, 2 * DAY);
    let e = store.entry(
        "unreadable-idle",
        100,
        Some(marker(Path::new("/gone"), SystemTime::now())),
    );
    age(&e, 2 * DAY);
    let unreadable = e.join(MARKER_FILE);
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    store.entry("garbage-fresh", 100, Some("\u{0}\u{1}garbage".into()));

    store.listing("before");
    let report = sweep(&store.root, &policy(true, 0));
    let after = store.listing("after");

    assert_eq!(after, set(["garbage-fresh"]));
    let reasons: BTreeMap<_, _> = report.invalid_markers.iter().cloned().collect();
    assert!(reasons[&store.name("garbage-idle")].contains("not JSON"));
    assert!(reasons[&store.name("garbage-fresh")].contains("not JSON"));
    assert!(reasons[&store.name("no-repository-idle")].contains("names no repository"));
    if !running_as_root() {
        assert!(reasons[&store.name("unreadable-idle")].contains("unreadable"));
    }
}

#[test]
fn an_idle_entry_that_cannot_be_read_in_full_is_kept() {
    if running_as_root() {
        return; // root reads through any permission, so nothing is unreadable
    }
    let mut store = Store::new();
    let e = store.entry("partly-unreadable", 100, None);
    age(&e, 2 * DAY);
    let sealed = e.join("target");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();

    sweep(&store.root, &policy(true, 1));
    // Unsealed before listing: the listing reads every byte to report `du`.
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
    let after = store.listing("after");

    assert_eq!(
        after,
        set(["partly-unreadable"]),
        "an unread subtree could hide a recent write"
    );
}

#[test]
fn the_recheck_under_the_lock_keeps_an_entry_that_gained_a_marker() {
    let mut store = Store::new();
    let e = store.entry("adopted", 100, None);
    age(&e, 2 * DAY);
    assert_eq!(unmarked_removable(&e, HOUR), Ok(()));

    std::fs::write(e.join(MARKER_FILE), marker(&store.base, SystemTime::now())).unwrap();
    age(&e, 2 * DAY);

    assert!(unmarked_removable(&e, HOUR).is_err());
}

#[test]
fn opening_an_entry_writes_its_marker_and_nothing_else() {
    let store = Store::new();
    let checkout = store.base.join("repo");
    std::fs::create_dir_all(&checkout).unwrap();

    for _ in 0..3 {
        let (entry, guard) = open_entry(&store.root, &checkout).unwrap().unwrap();
        let files: Vec<String> = std::fs::read_dir(&entry)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files, vec![MARKER_FILE.to_string()], "no temporary left");
        assert!(read_marker(&entry).is_some());
        drop(guard);
    }
}
