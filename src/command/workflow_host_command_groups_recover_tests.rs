use super::*;
use archon_shell::process_tree::Pinned;
use std::os::unix::process::CommandExt;
use std::time::Duration;

fn child() -> std::process::Child {
    let mut command = archon_shell::spawn::command("sleep");
    command.arg("30");
    // SAFETY: setsid takes no pointers and gives the child its own scope.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command.spawn().unwrap()
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct RecordedChild(Pinned);
impl Drop for RecordedChild {
    fn drop(&mut self) {
        archon_shell::process_tree::deliver(self.0, libc::SIGKILL);
    }
}

#[test]
#[ignore = "fixture subprocess; its parent kills it after recording its child"]
fn host_command_resume_fixture_owner() {
    let Some(root) = std::env::var_os("ARCHON_TEST_GROUP_RECOVERY_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let _lease = crate::command::workflow_executor_lease::acquire(&root, "TASK-X-001").unwrap();
    _lease.record_executor().unwrap();
    let child = child();
    let pid = child.id();
    let record = record_group(
        &root.join(GROUP_RECORDS_DIR),
        pid,
        pid,
        Some(pid),
        None,
        "TASK-X-001",
    )
    .unwrap();
    std::fs::write(root.join("ready"), record.path().to_str().unwrap()).unwrap();
    let _held = (record, child);
    std::thread::sleep(Duration::from_secs(30));
}

async fn recover(escaped: bool, reused: bool) {
    tokio::time::timeout(Duration::from_secs(15), async move {
        let run = tempfile::tempdir().unwrap();
        let module = module_path!();
        let module = module.split_once("::").map_or(module, |(_, rest)| rest);
        let mut owner = Child(
            archon_shell::spawn::command(std::env::current_exe().unwrap())
                .args([
                    format!("{module}::host_command_resume_fixture_owner"),
                    "--exact".into(),
                    "--ignored".into(),
                    "--test-threads=1".into(),
                ])
                .env("ARCHON_TEST_GROUP_RECOVERY_ROOT", run.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let ready = run.path().join("ready");
        while !ready.exists() {
            assert!(
                owner.0.try_wait().unwrap().is_none(),
                "fixture owner exited before recording its child"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let path = PathBuf::from(std::fs::read_to_string(ready).unwrap());
        let mut record: HostCommandGroupRecord =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let pid = record.pid;
        let start = record.leader_start.unwrap();
        let _child = RecordedChild(Pinned { pid, start });
        assert_eq!(record.host_pid, owner.0.id());
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        let lease =
            crate::command::workflow_executor_lease::acquire(run.path(), "TASK-X-001").unwrap();
        assert_eq!(lease.previous_executor().unwrap().pid, record.host_pid);
        if escaped {
            record.pgid = record.host_pid;
            record.session = None;
            record.leader_start = None;
            record.survivors = vec![(pid, start)];
        }
        if reused {
            record.leader_start = Some(start + 1);
        }
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        std::fs::write(
            path.with_extension("pending"),
            guard::supervised_marker(&record).unwrap(),
        )
        .unwrap();
        assert!(require_no_running_groups_read_only(run.path(), "TASK-X-001").is_err() || reused);
        let result = settle_before_resume(run.path(), "TASK-X-001", &lease, || Ok(())).await;
        if result.is_err() {
            eprintln!(
                "recorded pid {pid}, start {start}, identity after refusal: {:?}",
                archon_shell::process_tree::identity_of(pid)
            );
            eprintln!(
                "record: {:?}; marker: {:?}",
                std::fs::read_to_string(&path),
                std::fs::read_to_string(path.with_extension("pending"))
            );
        }
        assert!(
            result.is_ok(),
            "exclusive resume must settle its own recorded child: {result:?}"
        );
        assert_eq!(result.unwrap().len(), 1);
        if reused {
            assert_eq!(
                archon_shell::process_tree::identity_of(pid).unwrap(),
                Some(start),
                "a reused identity must not be killed"
            );
        } else {
            assert!(
                archon_shell::process_tree::identity_of(pid)
                    .unwrap()
                    .is_none(),
                "the recorded child must be confirmed dead"
            );
        }
        assert!(!path.exists());
        assert!(!path.with_extension("pending").exists());
    })
    .await
    .expect("recorded-child recovery must finish");
}

#[tokio::test]
async fn host_command_resume_settles_recorded_live_child_under_lease() {
    recover(false, false).await;
    recover(false, true).await;
}

#[tokio::test]
async fn host_command_resume_settles_recorded_escaped_identity_under_lease() {
    recover(true, false).await;
}

thread_local! {
    static CHECKPOINT_FAILURE: std::cell::Cell<Option<(Pinned, i32)>> = const { std::cell::Cell::new(None) };
}

pub(super) fn checkpoint(pins: &[Pinned]) -> std::io::Result<()> {
    if let Some((pin, errno)) = CHECKPOINT_FAILURE.get()
        && pins.contains(&pin)
    {
        CHECKPOINT_FAILURE.set(None);
        return Err(std::io::Error::from_raw_os_error(errno));
    }
    Ok(())
}

fn tree(root: &Path, exit: bool) -> (Child, RecordedChild, GroupRecordGuard) {
    let mut command = archon_shell::spawn::command("sh");
    command.args([
        "-c",
        if exit {
            "sleep 30 & echo $! > ready; read value"
        } else {
            "sleep 30 & echo $! > ready; wait"
        },
    ]);
    command
        .current_dir(root)
        .stdin(std::process::Stdio::piped());
    // SAFETY: setsid takes no pointers and gives the fixture its own scope.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = Child(command.spawn().unwrap());
    let guard = record_group(
        &root.join(GROUP_RECORDS_DIR),
        child.0.id(),
        child.0.id(),
        Some(child.0.id()),
        None,
        "TASK-X-001",
    )
    .unwrap();
    let ready = root.join("ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() || std::fs::read_to_string(&ready).unwrap().trim().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture must record its child"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = std::fs::read_to_string(ready)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let descendant = RecordedChild(Pinned {
        pid,
        start: archon_shell::process_tree::start_of(pid).unwrap(),
    });
    (child, descendant, guard)
}

#[tokio::test]
async fn groups_recover_absent_reused_leader_never_owns_numeric_scope_members() {
    tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(|| {
            let run = tempfile::tempdir().unwrap();
            let (mut replacement, descendant, guard) = tree(run.path(), true);
            drop(replacement.0.stdin.take());
            replacement.0.wait().unwrap();
            let mut record = guard.record.clone();
            record.file = Some(guard.path().to_owned());
            record.leader_start = Some(record.leader_start.unwrap() + 1);
            let retained = Default::default();
            let result = terminate_recorded(&mut record, &retained);
            assert_eq!(
                archon_shell::process_tree::identity_of(descendant.0.pid).unwrap(),
                Some(descendant.0.start),
                "numeric membership after an absent reused leader never authorizes a signal"
            );
            assert!(result.is_err(), "an ambiguous scope must retain its fence");
            assert!(guard.path().exists());
        }),
    )
    .await
    .expect("recovery must return promptly")
    .unwrap();
}

#[tokio::test]
async fn groups_recover_adoption_checkpoint_failure_stops_all_signals_and_keeps_typed_fault() {
    tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(|| {
            for errno in [libc::EIO, libc::ENOSPC] {
                let run = tempfile::tempdir().unwrap();
                let (mut root, descendant, guard) = tree(run.path(), false);
                let root_pin = Pinned {
                    pid: root.0.id(),
                    start: guard.record.leader_start.unwrap(),
                };
                let mut record = guard.record.clone();
                record.file = Some(guard.path().to_owned());
                record.survivors = vec![(root_pin.pid, root_pin.start)];
                record.host_pid = root_pin.pid;
                record.host_start = Some(root_pin.start);
                std::fs::write(guard.path(), serde_json::to_vec(&record).unwrap()).unwrap();
                CHECKPOINT_FAILURE.set(Some((descendant.0, errno)));
                let retained = Default::default();
                let result = terminate_recorded(&mut record, &retained);
                assert_eq!(
                    archon_shell::process_tree::identity_of(descendant.0.pid).unwrap(),
                    Some(descendant.0.start),
                    "an uncheckpointed adoption must never be signaled"
                );
                assert_eq!(
                    archon_shell::process_tree::identity_of(root_pin.pid).unwrap(),
                    Some(root_pin.start),
                    "signaling must stop at the first checkpoint failure"
                );
                let error = result.unwrap_err();
                let diagnostic = format!("{error:#}");
                assert!(
                    diagnostic
                        .contains(&guard.path().with_extension("pending").display().to_string())
                );
                if errno == libc::ENOSPC {
                    assert!(diagnostic.contains("free disk space"), "{diagnostic}");
                }
                assert_eq!(
                    crate::command::workflow_host_retry::transient(&error),
                    errno == libc::EIO,
                    "checkpoint fault must retain its typed class: {error:#}"
                );
                let marker = guard::pending_record(&guard.path().with_extension("pending"))
                    .unwrap()
                    .unwrap();
                assert!(marker.record.stalled);
                assert!(marker.record.survivors_unknown);
                assert!(!marker.record.teardown_complete);
                assert!(
                    marker
                        .record
                        .survivors
                        .contains(&(root_pin.pid, root_pin.start))
                );
                root.0.kill().unwrap();
                root.0.wait().unwrap();
                let (running, ended) = left_groups_read_only(run.path()).unwrap();
                assert!(
                    ended.is_empty(),
                    "a failed adoption must not be healed as a crash"
                );
                assert_eq!(running.len(), 1);
                assert!(running[0].survivors_unknown);
                terminate_recorded(&mut record, &retained).unwrap();
                assert!(
                    archon_shell::process_tree::identity_of(descendant.0.pid)
                        .unwrap()
                        .is_none()
                );
            }
        }),
    )
    .await
    .expect("recovery must return promptly")
    .unwrap();
}
