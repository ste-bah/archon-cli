//! Every test here runs against a fresh temporary directory. Nothing in this
//! file may name, read or remove a real cache store.

use super::*;
use cache_gc_entry::MARKER_FILE;
use std::fs::OpenOptions;

/// A policy that collects and caps, with no interval guard in the way.
fn strict(max_bytes: u64) -> CacheGcPolicy {
    CacheGcPolicy {
        collect_dead_entries: true,
        max_bytes,
        interval: Duration::from_secs(0),
    }
}

/// Create an entry for `checkout` holding `bytes` of payload, last used at
/// `last_used`. Written directly so tests pin the on-disk format rather than
/// inheriting whatever the writer happens to produce.
fn seed_entry(root: &Path, checkout: &Path, bytes: usize, last_used: &str) -> PathBuf {
    let entry = root.join(entry_name(checkout));
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("payload"), vec![b'x'; bytes]).unwrap();
    std::fs::write(
        entry.join(MARKER_FILE),
        serde_json::json!({
            "repository": checkout.to_string_lossy(),
            "created_at": last_used,
            "last_used_at": last_used,
        })
        .to_string(),
    )
    .unwrap();
    entry
}

/// A checkout directory that exists.
fn live_checkout(base: &Path, name: &str) -> PathBuf {
    let path = base.join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn entry_whose_checkout_is_gone_is_collected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "gone");
    let entry = seed_entry(&root, &checkout, 16, "2026-01-01T00:00:00Z");
    std::fs::remove_dir_all(&checkout).unwrap();

    let report = sweep(&root, &strict(0));

    assert!(!entry.exists(), "dead entry should be removed");
    assert_eq!(report.collected, vec![entry_name(&checkout)]);
    assert_eq!(report.evicted, Vec::<String>::new());
}

#[test]
fn entry_whose_checkout_still_exists_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "alive");
    let entry = seed_entry(&root, &checkout, 16, "2026-01-01T00:00:00Z");

    let report = sweep(&root, &strict(0));

    assert!(entry.exists(), "live entry must never be removed");
    assert!(report.collected.is_empty());
}

#[test]
fn entry_in_use_by_this_process_is_never_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    let checkout = live_checkout(tmp.path(), "busy");
    let (entry, guard) = open_entry(&root, &checkout)
        .unwrap()
        .expect("a fresh entry must be lockable");
    std::fs::write(entry.join("payload"), vec![b'x'; 4096]).unwrap();
    // The worst case: the checkout vanishes while a command is still running
    // in it, so the liveness rule alone would call the entry dead.
    std::fs::remove_dir_all(&checkout).unwrap();

    let report = sweep(&root, &strict(1));

    assert!(entry.exists(), "an entry in use must survive both passes");
    assert!(report.collected.is_empty());
    assert!(report.evicted.is_empty());
    assert_eq!(report.in_use, vec![entry_name(&checkout)]);
    drop(guard);
}

#[test]
fn entry_locked_by_another_holder_is_never_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "foreign");
    let entry = seed_entry(&root, &checkout, 16, "2026-01-01T00:00:00Z");
    std::fs::remove_dir_all(&checkout).unwrap();

    // A shared lock taken on a file description this module does not know
    // about — the same thing a second archon process would hold.
    let lock = root.join(format!("{}.lock", entry_name(&checkout)));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .unwrap();
    let rw = fd_lock::RwLock::new(file);
    let held_elsewhere = rw.try_read().unwrap();

    let report = sweep(&root, &strict(1));

    assert!(entry.exists(), "a foreign lock must block removal");
    assert!(report.collected.is_empty());
    assert_eq!(report.in_use, vec![entry_name(&checkout)]);
    drop(held_elsewhere);
}

#[test]
fn size_cap_evicts_least_recently_used_only_beyond_the_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let oldest = live_checkout(tmp.path(), "oldest");
    let middle = live_checkout(tmp.path(), "middle");
    let newest = live_checkout(tmp.path(), "newest");
    seed_entry(&root, &oldest, 10_000, "2026-01-01T00:00:00Z");
    seed_entry(&root, &middle, 10_000, "2026-02-01T00:00:00Z");
    seed_entry(&root, &newest, 10_000, "2026-03-01T00:00:00Z");

    // Room for two entries, not three.
    let report = sweep(&root, &strict(25_000));

    assert_eq!(report.evicted, vec![entry_name(&oldest)]);
    assert!(!root.join(entry_name(&oldest)).exists());
    assert!(root.join(entry_name(&middle)).exists());
    assert!(root.join(entry_name(&newest)).exists());
    assert!(report.managed_bytes <= 25_000);
}

#[test]
fn size_cap_never_evicts_an_entry_in_use() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    let busy = live_checkout(tmp.path(), "busy");
    let idle = live_checkout(tmp.path(), "idle");
    let (entry, guard) = open_entry(&root, &busy).unwrap().unwrap();
    std::fs::write(entry.join("payload"), vec![b'x'; 10_000]).unwrap();
    // Seeded as the least recently used, so the cap would take it first.
    let _ = std::fs::remove_file(entry.join(MARKER_FILE));
    std::fs::write(
        entry.join(MARKER_FILE),
        serde_json::json!({
            "repository": busy.to_string_lossy(),
            "created_at": "2020-01-01T00:00:00Z",
            "last_used_at": "2020-01-01T00:00:00Z",
        })
        .to_string(),
    )
    .unwrap();
    seed_entry(&root, &idle, 10_000, "2026-03-01T00:00:00Z");

    let report = sweep(&root, &strict(1));

    assert!(entry.exists(), "the held entry must survive the cap");
    assert_eq!(report.evicted, vec![entry_name(&idle)]);
    drop(guard);
}

#[test]
fn collection_can_be_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "gone");
    let entry = seed_entry(&root, &checkout, 16, "2026-01-01T00:00:00Z");
    std::fs::remove_dir_all(&checkout).unwrap();

    let report = sweep(
        &root,
        &CacheGcPolicy {
            collect_dead_entries: false,
            max_bytes: 0,
            interval: Duration::from_secs(0),
        },
    );

    assert!(entry.exists(), "collection off must remove nothing");
    assert!(report.collected.is_empty());
}

#[test]
fn size_cap_can_be_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let a = live_checkout(tmp.path(), "a");
    let b = live_checkout(tmp.path(), "b");
    seed_entry(&root, &a, 10_000, "2026-01-01T00:00:00Z");
    seed_entry(&root, &b, 10_000, "2026-02-01T00:00:00Z");

    let report = sweep(&root, &strict(0));

    assert!(report.evicted.is_empty(), "cap off must evict nothing");
    assert!(root.join(entry_name(&a)).exists());
    assert!(root.join(entry_name(&b)).exists());
}

#[test]
fn disabling_both_stops_maybe_sweep_before_it_touches_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    configure(CacheGcPolicy {
        collect_dead_entries: false,
        max_bytes: 0,
        interval: Duration::from_secs(0),
    });

    maybe_sweep(&root);

    assert!(!root.exists(), "a disabled sweep must not even stamp");
    configure(CacheGcPolicy::default());
}

#[test]
fn entry_without_a_marker_is_never_removed() {
    // A directory left by a build that predates the marker: the hash is
    // one-way, so nothing can say what it belongs to, and the process that
    // wrote it takes no lock. Leaking it is the only safe answer; a human
    // clears it deliberately.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let legacy = root.join("a".repeat(64));
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("payload"), vec![b'x'; 10_000]).unwrap();

    let report = sweep(&root, &strict(1));

    assert!(legacy.exists(), "a marker-less entry must survive");
    assert_eq!(report.unmanaged, 1);
    assert!(report.collected.is_empty());
    assert!(report.evicted.is_empty());
}

#[test]
fn sweep_ignores_anything_it_did_not_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let stranger = root.join("not-a-cache-entry");
    std::fs::create_dir_all(&stranger).unwrap();
    std::fs::write(stranger.join("payload"), vec![b'x'; 10_000]).unwrap();
    std::fs::write(root.join("loose-file"), "x").unwrap();

    let report = sweep(&root, &strict(1));

    assert!(stranger.exists());
    assert!(root.join("loose-file").exists());
    assert_eq!(report.unmanaged, 0);
    assert_eq!(report.managed_bytes, 0);
}

#[test]
fn an_interrupted_removal_is_finished_by_the_next_sweep() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let abandoned = root.join(format!("{}.7{}", "b".repeat(64), TRASH_SUFFIX));
    std::fs::create_dir_all(&abandoned).unwrap();
    std::fs::write(abandoned.join("payload"), "x").unwrap();

    sweep(&root, &strict(0));

    assert!(!abandoned.exists(), "committed removals must be finished");
}

#[test]
fn opening_an_entry_records_its_checkout_and_refreshes_last_use() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    let checkout = live_checkout(tmp.path(), "repo");

    let (entry, first) = open_entry(&root, &checkout).unwrap().unwrap();
    let created = read_marker(&entry).unwrap();
    drop(first);
    std::thread::sleep(Duration::from_millis(5));
    let (_, second) = open_entry(&root, &checkout).unwrap().unwrap();
    let reused = read_marker(&entry).unwrap();
    drop(second);

    assert_eq!(created.repository, checkout.to_string_lossy());
    assert_eq!(
        reused.created_at, created.created_at,
        "creation is recorded once"
    );
    assert!(
        reused.last_used_at > created.last_used_at,
        "every acquire refreshes last use: {} then {}",
        created.last_used_at,
        reused.last_used_at
    );
}

#[test]
fn the_interval_guard_admits_one_sweep_per_window() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    assert!(claim_sweep(root, Duration::from_secs(3600)));
    assert!(!claim_sweep(root, Duration::from_secs(3600)));
    assert!(claim_sweep(root, Duration::from_secs(0)));
}

#[cfg(unix)]
#[path = "cache_gc_lock_tests.rs"]
mod lock_tests;
