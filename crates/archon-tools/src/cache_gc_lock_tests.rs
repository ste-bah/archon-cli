//! How an entry's lock is taken, held and released, and which absences the
//! liveness rule refuses to trust. Split from `cache_gc_tests` for the
//! 500-line ceiling. Every test runs against a fresh temporary directory.

use super::super::cache_gc_entry::{lock_path, open_entry_waiting};
use super::super::*;
use std::fs::OpenOptions;

fn strict() -> CacheGcPolicy {
    CacheGcPolicy {
        collect_dead_entries: true,
        max_bytes: 0,
        interval: Duration::from_secs(0),
    }
}

fn live_checkout(base: &Path, name: &str) -> PathBuf {
    let path = base.join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Open descriptors in this process that refer to `path`.
///
/// Counts by what each descriptor names rather than by how many are open, so
/// other tests opening files concurrently cannot disturb the answer.
fn open_descriptors_naming(path: &Path) -> usize {
    let want = path.canonicalize().unwrap();
    // SAFETY: getdtablesize has no preconditions.
    let limit = unsafe { libc::getdtablesize() };
    (0..limit)
        .filter(|&fd| descriptor_path(fd).is_some_and(|p| p == want))
        .count()
}

#[cfg(target_os = "macos")]
fn descriptor_path(fd: i32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes into `buf`.
    let rc = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc == -1 {
        return None;
    }
    let len = buf.iter().position(|b| *b == 0)?;
    let text = std::str::from_utf8(&buf[..len]).ok()?;
    Path::new(text).canonicalize().ok()
}

#[cfg(not(target_os = "macos"))]
fn descriptor_path(fd: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .ok()?
        .canonicalize()
        .ok()
}

/// An exclusive lock on `lock` from a descriptor this module does not know
/// about — what a sweep holds while it decides and renames.
fn exclusive(lock: &Path) -> fd_lock::RwLock<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)
        .unwrap();
    fd_lock::RwLock::new(file)
}

#[test]
fn releasing_an_entry_closes_its_lock_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    let checkout = live_checkout(tmp.path(), "repo");
    let lock = lock_path(&root, &entry_name(&checkout));

    for _ in 0..20 {
        let (_, guard) = open_entry(&root, &checkout).unwrap().unwrap();
        assert_eq!(open_descriptors_naming(&lock), 1, "held: one descriptor");
        drop(guard);
    }

    assert_eq!(
        open_descriptors_naming(&lock),
        0,
        "every take/release cycle must close the lock file it opened"
    );
}

#[test]
fn sharing_an_entry_in_one_process_keeps_one_descriptor_until_the_last_release() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    let checkout = live_checkout(tmp.path(), "repo");
    let lock = lock_path(&root, &entry_name(&checkout));

    let (_, first) = open_entry(&root, &checkout).unwrap().unwrap();
    let (_, second) = open_entry(&root, &checkout).unwrap().unwrap();
    assert_eq!(open_descriptors_naming(&lock), 1);
    drop(first);
    assert!(
        exclusive(&lock).try_write().is_err(),
        "one user remains, so the lock must still be held"
    );
    drop(second);
    assert_eq!(open_descriptors_naming(&lock), 0);
    assert!(
        exclusive(&lock).try_write().is_ok(),
        "the last release must release the lock"
    );
}

#[test]
fn an_entry_a_sweep_holds_is_never_used_unlocked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "repo");
    let lock = lock_path(&root, &entry_name(&checkout));
    let mut sweeper = exclusive(&lock);
    let _held = sweeper.try_write().unwrap();

    let opened = open_entry_waiting(&root, &checkout, Duration::from_millis(60)).unwrap();

    assert!(opened.is_none(), "no lock, so the entry must not be used");
    assert!(
        !root.join(entry_name(&checkout)).exists(),
        "nothing may be created under a sweep's exclusive lock"
    );
    assert_eq!(open_descriptors_naming(&lock), 1, "only the sweep's own");
}

#[test]
fn a_user_waits_out_a_short_sweep_and_then_holds_the_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = live_checkout(tmp.path(), "repo");
    let lock = lock_path(&root, &entry_name(&checkout));
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let sweeper_lock = lock.clone();
    let sweeper = std::thread::spawn(move || {
        let mut rw = exclusive(&sweeper_lock);
        let held = rw.try_write().unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        drop(held);
    });
    locked_rx.recv().unwrap();

    let opened = open_entry_waiting(&root, &checkout, Duration::from_secs(5)).unwrap();
    sweeper.join().unwrap();

    let (entry, guard) = opened.expect("the sweep released within the bound");
    assert!(read_marker(&entry).is_some());
    assert!(
        exclusive(&lock).try_write().is_err(),
        "now held by the user"
    );
    drop(guard);
}

#[test]
fn a_checkout_on_an_unmounted_volume_is_not_dead() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("unleased");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = PathBuf::from("/Volumes/archon-test-never-mounted-7f3a9c/repo");
    let entry = root.join(entry_name(&checkout));
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(
        entry.join(super::super::cache_gc_entry::MARKER_FILE),
        serde_json::json!({
            "repository": checkout.to_string_lossy(),
            "created_at": "2026-01-01T00:00:00Z",
            "last_used_at": "2026-01-01T00:00:00Z",
        })
        .to_string(),
    )
    .unwrap();

    let report = sweep(&root, &strict());

    assert!(
        entry.exists(),
        "a detached disk is not proof the checkout is gone"
    );
    assert!(report.collected.is_empty());
}

#[test]
fn volume_root_names_only_the_volume() {
    assert_eq!(
        volume_root(Path::new("/Volumes/Work/a/b")),
        Some(PathBuf::from("/Volumes/Work"))
    );
    assert_eq!(volume_root(Path::new("/Volumes")), None);
    assert_eq!(volume_root(Path::new("/Users/x/Volumes/Work")), None);
    assert_eq!(volume_root(Path::new("relative/Volumes/Work")), None);
}

#[test]
fn a_leftover_directory_is_not_a_mounted_volume() {
    let tmp = tempfile::tempdir().unwrap();
    let stale = live_checkout(tmp.path(), "Detached");

    assert!(!volume_is_mounted(&stale), "same device as its parent");
    assert!(!volume_is_mounted(&tmp.path().join("absent")));
}
