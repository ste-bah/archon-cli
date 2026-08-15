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

/// The scratch build directory is a SIBLING of the worktree, not a child.
/// Inside it, removing the tree and discarding the build output would be one
/// irreversible step, and neither could be measured on its own.
#[test]
fn the_build_directory_is_not_inside_the_worktree() {
    let worktree = WorktreeManager::worktrees_dir().join("subagent-example");
    let scratch = WorktreeManager::scratch_target_dir("subagent-example");

    assert!(!scratch.starts_with(&worktree), "{scratch:?}");
    assert_eq!(scratch.parent(), worktree.parent());
}

#[test]
fn usage_totals_both_halves() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 200,
        build_bytes: 1_000,
    };
    assert_eq!(usage.total_bytes(), 1_200);
}

/// The build figure is the one that matters, so it is never folded into a
/// single number when present.
#[test]
fn a_build_directory_is_reported_separately() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 210 * 1024 * 1024,
        build_bytes: 9 * 1024 * 1024 * 1024,
    };
    let described = usage.describe();

    assert!(described.contains("MB"), "{described}");
    assert!(described.contains("build"), "{described}");
    assert!(described.contains("GB"), "{described}");
}

/// An edit-only agent has no build directory, and its summary should not
/// suggest otherwise.
#[test]
fn no_build_directory_reads_as_one_figure() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 1024,
        build_bytes: 0,
    };
    let described = usage.describe();

    assert!(!described.contains("build"), "{described}");
    assert!(described.contains("KB"), "{described}");
}

#[test]
fn small_sizes_stay_in_bytes() {
    let usage = WorktreeDiskUsage {
        checkout_bytes: 512,
        build_bytes: 0,
    };
    assert_eq!(usage.describe(), "512 B");
}
