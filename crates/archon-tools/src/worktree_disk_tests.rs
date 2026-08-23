//! Disk-accounting tests (#184 M3).

use super::*;

#[test]
fn an_absent_directory_measures_zero_rather_than_failing() {
    let missing = std::env::temp_dir().join("archon-worktree-disk-no-such-dir");
    assert_eq!(directory_size(&missing), 0);
}

#[test]
fn a_directory_size_counts_nested_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("a.txt"), vec![0u8; 100]).expect("write");
    let nested = dir.path().join("deep").join("deeper");
    std::fs::create_dir_all(&nested).expect("mkdir");
    std::fs::write(nested.join("b.bin"), vec![0u8; 250]).expect("write");

    assert_eq!(directory_size(dir.path()), 350);
}

/// The lease-slot name has one definition, and this is the round trip that
/// keeps the reader and the writer from drifting apart.
#[test]
fn a_lease_slot_directory_name_round_trips() {
    let root = std::path::Path::new("/pool");
    for slot in [0usize, 1, 7, 42] {
        let dir = crate::build_cache_env::lease_slot_dir(root, slot);
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        assert_eq!(
            crate::build_cache_env::lease_slot_of_dir_name(name),
            Some(slot),
            "{name}"
        );
    }
}

/// Whatever else an operator leaves in the pool root is not a slot, and must
/// not be counted as one.
#[test]
fn a_name_that_is_not_a_slot_is_not_read_as_one() {
    for name in ["build-cache", "build-cache-", "build-cache-x", "target", ""] {
        assert_eq!(
            crate::build_cache_env::lease_slot_of_dir_name(name),
            None,
            "{name}"
        );
    }
}

/// The regression this module was rewritten for: build output lives in slot
/// directories under the pool root, and the report has to find it there.
#[test]
fn the_build_cache_is_measured_from_the_slot_directories_on_disk() {
    let root = tempfile::tempdir().expect("temp dir");
    for (slot, bytes) in [(0usize, 100usize), (1, 250)] {
        let dir = crate::build_cache_env::lease_slot_dir(root.path(), slot).join("cargo");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("artifact.bin"), vec![0u8; bytes]).expect("write");
    }
    // Not a slot: it must be visible on disk and absent from the total.
    std::fs::create_dir_all(root.path().join("notes")).expect("mkdir");
    std::fs::write(root.path().join("notes").join("x"), vec![0u8; 9_999]).expect("write");

    let usage = BuildCacheUsage::measure(root.path());

    assert_eq!(usage.slots, vec![(0, 100), (1, 250)]);
    assert_eq!(usage.total_bytes(), 350);
    assert!(
        usage.describe().contains("2 build-cache slot(s)"),
        "{}",
        usage.describe()
    );
}

/// A machine that has never run a workflow has no pool, and the listing must
/// say nothing rather than print an empty figure.
#[test]
fn an_absent_pool_root_measures_as_empty() {
    let missing = std::env::temp_dir().join("archon-build-cache-no-such-pool");
    let usage = BuildCacheUsage::measure(&missing);

    assert!(usage.is_empty());
    assert_eq!(usage.total_bytes(), 0);
}

/// The pool root is one path, and both the process that creates it and the
/// process that reports it have to name it the same way.
#[test]
fn the_pool_root_sits_under_the_worktrees_directory() {
    let root = WorktreeManager::build_cache_root();

    assert_eq!(
        root.parent(),
        Some(WorktreeManager::worktrees_dir().as_path())
    );
    assert_eq!(
        root.file_name().and_then(|name| name.to_str()),
        Some("build-cache")
    );
}

#[test]
fn a_checkout_is_described_in_human_units() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 210 * 1024 * 1024,
    };
    assert!(usage.describe().contains("MB"), "{}", usage.describe());
    assert_eq!(usage.total_bytes(), 210 * 1024 * 1024);
}

#[test]
fn small_sizes_stay_in_bytes() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 512,
    };
    assert_eq!(usage.describe(), "512 B");
}
