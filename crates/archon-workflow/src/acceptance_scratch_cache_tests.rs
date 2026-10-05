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

#[test]
fn missing_and_corrupt_records_forget_builds_but_valid_empty_records_do_not() {
    for bytes in [None, Some("invalid"), Some("{}")] {
        let cache = tempfile::tempdir().unwrap();
        killed_holder(cache.path(), &[None, Some(dead_group())]);
        let record = cache.path().join(RECORD);
        match bytes {
            None => std::fs::remove_file(record).unwrap(),
            Some(bytes) => std::fs::write(record, bytes).unwrap(),
        }
        assert_eq!(
            reacquired(cache.path()).generation,
            if bytes == Some("{}") { 0 } else { 1 }
        );
    }
}

#[test]
fn detached_writer_prevents_warm_reuse_until_it_exits() {
    for alive in [true, false] {
        let cache = tempfile::tempdir().unwrap();
        killed_holder(cache.path(), &[None, Some(dead_group())]);
        let target = cache.path().join("target-0");
        let mut command = std::process::Command::new("sh");
        command
            .args([
                "-c",
                "echo ready > ready; while :; do echo x >> held; sleep 0.1; done",
            ])
            .current_dir(&target);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let start = std::time::Instant::now();
        while !target.join("ready").exists() {
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !alive {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            child.wait().unwrap();
            // SIGKILL is asynchronous: a descendant (the loop's `sleep`) can
            // still hold the target as its cwd until it has exited. Wait for
            // the release the cache itself probes, not for zombies to be reaped.
            let start = std::time::Instant::now();
            while !target_idle(&target) {
                assert!(
                    start.elapsed().as_secs() < 10,
                    "killed writer never released the target"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let lease = reacquired(cache.path());
        if alive {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
        }
        child.wait().unwrap();
        assert_eq!(lease.generation, u64::from(alive));
    }
}

const CONFINED_PATH_ENV: &str = "ARCHON_TEST_CONFINED_PATH_CACHE";

/// Issue 311: the guardian runs with its environment cleared and PATH set to
/// the policy's toolchain path (for example `/usr/bin:/bin`), where an
/// sbin-installed `lsof` is not on PATH. The idle check must still find it,
/// or every slot left behind looks busy and the warm target is discarded.
/// Run in a child test process with exactly that environment.
#[test]
fn a_left_slot_keeps_the_warm_target_under_a_confined_path() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "acceptance_scratch::cache::tests::confined_path_child_keeps_the_warm_target",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(CONFINED_PATH_ENV, "1")
        .status()
        .unwrap();
    assert!(status.success(), "the confined-PATH child failed: {status}");
}

#[test]
#[ignore = "child process of a_left_slot_keeps_the_warm_target_under_a_confined_path"]
fn confined_path_child_keeps_the_warm_target() {
    if std::env::var_os(CONFINED_PATH_ENV).is_none() {
        return;
    }
    assert_eq!(std::env::var("PATH").unwrap(), "/usr/bin:/bin");
    let cache = tempfile::tempdir().unwrap();
    killed_holder(cache.path(), &[None, Some(dead_group())]);
    let lease = reacquired(cache.path());
    assert_eq!(lease.generation, 0, "the cache was forgotten");
    assert!(lease.target().join("warm-artifact").exists());
    let idle = tempfile::tempdir().unwrap();
    assert!(target_idle(idle.path()), "an idle target read as busy");
}

/// Issue 321: no `lsof` anywhere, or one that cannot run, proves nothing,
/// so an idle target counts as busy, never as idle.
#[test]
fn an_idle_check_without_a_runnable_lsof_reads_busy() {
    let idle = tempfile::tempdir().unwrap();
    let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "no executable lsof");
    assert!(!lsof_says_idle(Err(missing), idle.path()));
    let plain = idle.path().join("plain-lsof");
    std::fs::write(&plain, "").unwrap();
    assert!(!lsof_says_idle(Ok(plain), idle.path()));
    assert!(!lsof_says_idle(
        Ok(idle.path().join("gone/lsof")),
        idle.path()
    ));
}
