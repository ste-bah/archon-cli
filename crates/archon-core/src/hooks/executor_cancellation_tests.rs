use super::*;

// Clean up the fixture's exact IDs even when the old executor fails the test.
struct FixturePids(Vec<i32>);
impl Drop for FixturePids {
    fn drop(&mut self) {
        for pid in &self.0 {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
    }
}

async fn read_pid(path: &Path) -> i32 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("pid fixture ready")
}

async fn cancellation(parent_exits: bool) {
    let dir = tempfile::tempdir().unwrap();
    let command = format!(
        "echo $$ > parent.pid; sleep 600 & echo $! > descendant.pid; {}",
        if parent_exits { "exit 0" } else { "wait" }
    );
    let cwd = dir.path().to_owned();
    let task = tokio::spawn(async move {
        run_command(&command, b"{}", &cwd, "cancel", "PostToolUse", 60).await
    });
    let parent = read_pid(&dir.path().join("parent.pid")).await;
    let descendant = read_pid(&dir.path().join("descendant.pid")).await;
    let _cleanup = FixturePids(vec![parent, descendant]);
    if parent_exits {
        tokio::time::timeout(Duration::from_secs(5), async {
            while unsafe { libc::kill(parent, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("parent exit observed before cancellation");
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    for _ in 0..100 {
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("descendant {descendant} survived cancellation (parent_exits={parent_exits})");
}

#[tokio::test]
async fn cancellation_before_parent_exit_kills_descendants() {
    cancellation(false).await;
}

#[tokio::test]
async fn cancellation_after_parent_exit_kills_descendants() {
    cancellation(true).await;
}

#[tokio::test]
async fn dropping_unclaimed_spawn_result_kills_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let spawned = spawn_hook_process(
        "echo $$ > parent.pid; sleep 600 & echo $! > descendant.pid; wait",
        dir.path(),
        "cancel",
        "PostToolUse",
    )
    .await
    .unwrap();
    let parent = read_pid(&dir.path().join("parent.pid")).await;
    let descendant = read_pid(&dir.path().join("descendant.pid")).await;
    let _cleanup = FixturePids(vec![parent, descendant]);
    drop(spawned);
    for _ in 0..100 {
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("unclaimed spawn left descendant alive");
}

#[tokio::test]
async fn dropping_reader_owner_aborts_both_pending_readers() {
    let (stdout_writer, stdout_pipe) = tokio::io::duplex(64);
    let (stderr_writer, stderr_pipe) = tokio::io::duplex(64);
    let budget = Arc::new(AtomicUsize::new(64));
    let window = NoProgressWindow::new(Duration::from_secs(60));
    let stdout = drain_pipe(Some(stdout_pipe), Arc::clone(&budget), Arc::clone(&window));
    let stderr = drain_pipe(Some(stderr_pipe), budget, window);
    let owner = PipeReaders::new(&stdout, &stderr);
    drop(owner);
    tokio::task::yield_now().await;
    assert!(
        stdout.is_finished() && stderr.is_finished(),
        "pipe readers detached instead of aborted"
    );
    drop((stdout_writer, stderr_writer));
}

/// An unrelated process group, standing in for one that took the hook
/// group's ID after the leader was reaped and the group ended.
fn unrelated_group() -> tokio::process::Child {
    use std::os::unix::process::CommandExt;
    let mut command = archon_shell::spawn::command("sleep");
    command
        .arg("600")
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    command.spawn().expect("unrelated group")
}

/// A hook whose leader has exited and been reaped, with its guard pointed
/// at `unrelated`, as if the ID had been reused.
async fn reaped_hook_retargeted(unrelated: &tokio::process::Child) -> OwnedChild {
    let dir = tempfile::tempdir().unwrap();
    let mut hook = spawn_hook_process("exit 0", dir.path(), "reuse", "PostToolUse")
        .await
        .unwrap()
        .child;
    hook.wait().await.expect("leader reaped");
    hook.retarget_group_for_test(unrelated.id().expect("unrelated pid"));
    hook
}

fn assert_alive(unrelated: &mut tokio::process::Child) {
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "a reused process group ID was signalled"
    );
}

#[tokio::test]
async fn cleanup_after_the_reap_never_kills_a_reused_group() {
    let mut unrelated = unrelated_group();
    let mut hook = reaped_hook_retargeted(&unrelated).await;
    let cleanup_error = terminate_process_tree(&mut hook).await;
    assert_eq!(
        cleanup_error, None,
        "a reused ID means the group already ended"
    );
    assert_alive(&mut unrelated);
}

#[tokio::test]
async fn dropping_a_reaped_hook_never_kills_a_reused_group() {
    let mut unrelated = unrelated_group();
    let hook = reaped_hook_retargeted(&unrelated).await;
    drop(hook);
    assert_alive(&mut unrelated);
}

#[tokio::test]
async fn a_reused_group_id_counts_as_the_group_ending() {
    let mut unrelated = unrelated_group();
    let mut hook = reaped_hook_retargeted(&unrelated).await;
    let waited = tokio::time::timeout(Duration::from_secs(2), hook.wait_group_empty()).await;
    assert!(
        matches!(waited, Ok(Ok(()))),
        "waited on a reused group: {waited:?}"
    );
    drop(hook);
    assert_alive(&mut unrelated);
}

#[tokio::test]
async fn cleanup_after_the_reap_still_kills_the_hooks_own_group() {
    let dir = tempfile::tempdir().unwrap();
    let command = "sleep 600 </dev/null >/dev/null 2>&1 & echo $! > descendant.pid; exit 0";
    let mut hook = spawn_hook_process(command, dir.path(), "own", "PostToolUse")
        .await
        .unwrap()
        .child;
    hook.wait().await.expect("leader reaped");
    let descendant = read_pid(&dir.path().join("descendant.pid")).await;
    let _cleanup = FixturePids(vec![descendant]);
    assert_eq!(terminate_process_tree(&mut hook).await, None);
    for _ in 0..100 {
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("descendant {descendant} survived cleanup after the leader was reaped");
}
