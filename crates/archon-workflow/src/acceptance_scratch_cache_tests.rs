//! A slot its holder never tore down keeps the warm target only when it
//! cannot have spoiled it (Issue 255), on a real cache directory.
use super::*;
use std::os::unix::process::CommandExt;

const LIMIT: u64 = 1 << 40;

/// A holder that built into generation 0 and was killed before teardown:
/// its slot holds one tracked file, given its recorded time, and the
/// registry lines `groups` write.
fn killed_holder(dir: &Path, groups: &[Option<i32>]) -> PathBuf {
    let mut lease = Lease::acquire(dir, LIMIT).unwrap();
    assert_eq!(lease.generation, 0);
    std::fs::write(lease.target().join("warm-artifact"), "built").unwrap();
    let slot = lease.slot();
    std::fs::create_dir_all(slot.join("repo/src")).unwrap();
    std::fs::write(slot.join("repo/src/lib.rs"), "pub fn f() {}").unwrap();
    lease
        .stabilize(&[("repo", &slot.join("repo"))], &["src/lib.rs"])
        .unwrap();
    open_group_registry(&slot).unwrap();
    for group in groups {
        record_group(&slot, *group).unwrap();
    }
    drop(lease); // the lock goes with the process; the slot stays
    slot
}

/// A process group that has already exited and been reaped.
fn dead_group() -> i32 {
    let mut child = std::process::Command::new("true")
        .process_group(0)
        .spawn()
        .unwrap();
    let id = child.id() as i32;
    child.wait().unwrap();
    id
}

fn reacquired(dir: &Path) -> Lease {
    Lease::acquire(dir, LIMIT).unwrap()
}

#[test]
fn a_slot_whose_groups_are_gone_and_files_intact_keeps_the_warm_target() {
    let cache = tempfile::tempdir().unwrap();
    killed_holder(cache.path(), &[None, Some(dead_group())]);
    let lease = reacquired(cache.path());
    assert_eq!(lease.generation, 0, "the cache is not forgotten");
    assert!(lease.target().join("warm-artifact").exists());
}

#[test]
fn a_slot_that_never_spawned_a_check_keeps_the_warm_target() {
    let cache = tempfile::tempdir().unwrap();
    let slot = killed_holder(cache.path(), &[]);
    // Killed while it was still copying: nothing it holds was built from.
    std::fs::remove_dir_all(slot.join("repo")).unwrap();
    assert_eq!(reacquired(cache.path()).generation, 0);
}

#[test]
fn a_group_still_alive_forgets_every_build() {
    let cache = tempfile::tempdir().unwrap();
    let mut orphan = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    killed_holder(cache.path(), &[None, Some(orphan.id() as i32)]);
    let lease = reacquired(cache.path());
    let _ = orphan.kill();
    let _ = orphan.wait();
    assert_eq!(lease.generation, 1, "an orphan may still write the target");
    assert!(!cache.path().join("target-0").exists());
}

#[test]
fn a_touched_tracked_file_forgets_every_build() {
    let cache = tempfile::tempdir().unwrap();
    let slot = killed_holder(cache.path(), &[None, Some(dead_group())]);
    std::fs::write(slot.join("repo/src/lib.rs"), "pub fn g() {}").unwrap();
    assert_eq!(reacquired(cache.path()).generation, 1);
}

#[test]
fn an_unrecorded_spawn_or_a_missing_registry_forgets_every_build() {
    let cache = tempfile::tempdir().unwrap();
    killed_holder(cache.path(), &[None]);
    assert_eq!(
        reacquired(cache.path()).generation,
        1,
        "a spawn with no group"
    );
    let cache = tempfile::tempdir().unwrap();
    let slot = killed_holder(cache.path(), &[]);
    std::fs::remove_file(slot.join(GROUP_REGISTRY)).unwrap();
    assert_eq!(
        reacquired(cache.path()).generation,
        1,
        "a slot from before the registry"
    );
}

#[test]
fn another_generations_leftover_slot_does_not_forget_this_one() {
    let cache = tempfile::tempdir().unwrap();
    killed_holder(cache.path(), &[None, Some(dead_group())]);
    let stray = cache.path().join("scratch-7");
    std::fs::create_dir_all(&stray).unwrap();
    assert_eq!(reacquired(cache.path()).generation, 0);
}
