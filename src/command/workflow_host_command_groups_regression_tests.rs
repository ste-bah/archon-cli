use super::*;

fn unknown_remedy(mode: &str) {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let mut path = guard.path().to_path_buf();
    guard.keep(None);
    if mode != "record" {
        std::fs::write(
            path.with_extension("pending"),
            serde_json::to_vec(&read(&path)).unwrap(),
        )
        .unwrap();
    }
    let pending = path.with_extension("pending");
    if mode == "unknown-suffix" {
        let unknown = unknown_path(&path);
        std::fs::rename(&path, &unknown).unwrap();
        path = unknown;
    }
    let note = require_no_running_groups(run.path(), "run")
        .unwrap_err()
        .to_string();
    assert!(note.contains(&path.display().to_string()), "{note}");
    assert!(!note.contains("stop that group"), "{note}");
    if mode != "record" {
        assert!(note.contains(&pending.display().to_string()), "{note}");
    }
}
#[test]
fn unknown_record_remedy_names_files() {
    unknown_remedy("record");
}
#[test]
fn legacy_unknown_remedy_names_both_files() {
    unknown_remedy("legacy");
}
#[test]
fn renamed_unknown_remedy_names_both_files() {
    unknown_remedy("unknown-suffix");
}

#[test]
fn healing_permission_error_names_the_record_path() {
    use std::os::unix::fs::PermissionsExt;
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    guard.keep(Some(&[]));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = left_groups(run.path());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let note = result.unwrap_err().to_string();
    assert!(note.contains(&path.display().to_string()), "{note}");
}

#[test]
fn crash_before_keep_reads_identities_from_the_supervision_marker() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let (mut child, identity) = sleeper();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    let mut record = read(&path);
    record.host_pid = ended_group();
    record.host_start = None;
    record.survivors = vec![identity];
    // This fixture models a completed identity checkpoint; initial records
    // are intentionally incomplete until durable completion.
    record.survivors_unknown = false;
    std::fs::write(
        &path,
        serde_json::to_vec(&HostCommandGroupRecord {
            survivors: vec![],
            ..record.clone()
        })
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        path.with_extension("pending"),
        guard::supervised_marker(&record).unwrap(),
    )
    .unwrap();
    owner::release(&path.with_extension("pending"));
    let result = require_no_running_groups(run.path(), "run");
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        result.is_err(),
        "a marker's escaped identity was lost on crash"
    );
    drop(guard);
}

#[test]
fn crash_mid_freeze_keeps_incomplete_marker_scope_unknown() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    let mut record = read(&path);
    record.host_pid = ended_group();
    record.host_start = None;
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    record.survivors_unknown = true;
    std::fs::write(
        path.with_extension("pending"),
        guard::supervised_marker(&record).unwrap(),
    )
    .unwrap();
    owner::release(&path.with_extension("pending"));
    let result = require_no_running_groups(run.path(), "run");
    drop(guard);
    assert!(
        result.is_err(),
        "an incomplete freeze was healed after its owner exited"
    );
}

#[test]
fn crash_after_kill_before_reap_keeps_the_last_identity_checkpoint() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let (mut child, identity) = sleeper();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    let mut record = read(&path);
    record.host_pid = ended_group();
    record.host_start = None;
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    // Model a process whose kill was requested but has not yet finished.
    record.survivors = vec![identity];
    // This fixture models a completed identity checkpoint; initial records
    // are intentionally incomplete until durable completion.
    record.survivors_unknown = false;
    std::fs::write(
        path.with_extension("pending"),
        guard::supervised_marker(&record).unwrap(),
    )
    .unwrap();
    owner::release(&path.with_extension("pending"));
    let result = require_no_running_groups(run.path(), "run");
    let _ = child.kill();
    let _ = child.wait();
    let healed = require_no_running_groups(run.path(), "run");
    drop(guard);
    assert!(
        result.is_err(),
        "kill request was mistaken for exit confirmation"
    );
    assert!(healed.is_ok(), "gone identities should heal");
}

#[test]
fn a_real_executor_crash_retains_a_descendant_that_left_its_session() {
    use archon_shell::process_tree::{Pinned, deliver, identity_of};
    struct Children {
        host: std::process::Child,
        pins: Vec<Pinned>,
    }
    impl Drop for Children {
        fn drop(&mut self) {
            let _ = self.host.kill();
            let _ = self.host.wait();
            for &pin in &self.pins {
                deliver(pin, libc::SIGKILL);
            }
        }
    }
    let run = tempfile::tempdir().unwrap();
    let mut children = Children {
        host: archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "command::workflow_host_command_groups::tests::regression_tests::supervision_fixture"])
            .env("HOST_COMMAND_CRASH_FIXTURE", run.path())
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap(),
        pins: Vec::new(),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let escaped = loop {
        if let Ok(pid) = std::fs::read_to_string(run.path().join("escaped"))
            && let Ok(pid) = pid.parse::<u32>()
            && let Ok(Some(start)) = identity_of(pid)
        {
            break Pinned { pid, start };
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture never spawned its descendant"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    children.pins.push(escaped);
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let path = loop {
        let path = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "json")
                    && std::fs::read(path)
                        .ok()
                        .and_then(|bytes| {
                            serde_json::from_slice::<HostCommandGroupRecord>(&bytes).ok()
                        })
                        .is_some_and(|record| record.pid > 1 && record.leader_start.is_some())
            });
        if let Some(path) = path {
            break path;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture never registered its leader"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let record = read(&path);
    let leader = Pinned {
        pid: record.pid,
        start: record.leader_start.unwrap(),
    };
    children.pins.push(leader);
    let marker = path.with_extension("pending");
    // Finding the descendant is progress; marker persistence gets a fresh
    // no-progress window rather than the remainder of a total deadline.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let recorded = loop {
        let recorded = std::fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["supervised_record"]["survivors"].as_array().cloned())
            .is_some_and(|pins| {
                pins.iter()
                    .any(|pin| pin[0] == escaped.pid && pin[1] == escaped.start)
            });
        if recorded || std::time::Instant::now() >= deadline {
            break recorded;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    children.host.kill().unwrap();
    children.host.wait().unwrap();
    deliver(leader, libc::SIGKILL);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while group_running(leader.pid) != Some(false) {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture leader did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let refused = require_no_running_groups(run.path(), "run");
    assert!(
        recorded,
        "the escaped identity never reached disk before keep()"
    );
    assert!(
        refused.is_err(),
        "crash healing discarded a live escaped descendant"
    );
}

#[tokio::test]
#[ignore = "isolated executor crash fixture"]
async fn supervision_fixture() {
    use crate::command::workflow_host_command_catalog::ResolvedHostCommand;
    use crate::command::workflow_host_command_supervisor::{
        HostCommandControl, supervise_process_group,
    };
    let root = PathBuf::from(std::env::var_os("HOST_COMMAND_CRASH_FIXTURE").unwrap());
    let request = ResolvedHostCommand {
        command_id: "fixture".into(), program: "python3".into(),
        args: vec!["-c".into(), "import subprocess,time; p=subprocess.Popen(['sleep','30'],start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); open('escaped','w').write(str(p.pid)); time.sleep(30)".into()],
        cwd: root.clone(), environment: std::env::vars_os().filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value))).collect(),
        stdin: None, timeout_secs: 30, max_stdout_bytes: 4096, max_stderr_bytes: 4096,
        declared_write_set: Vec::new(), remediation_scopes: Default::default(),
    };
    let (control, _handle) = HostCommandControl::new();
    supervise_process_group(request, control, Some(&root.join(GROUP_RECORDS_DIR)))
        .await
        .unwrap();
}

#[path = "workflow_host_command_evidence_r2_tests.rs"]
mod r2_tests;

#[path = "workflow_host_command_remedy_r3_tests.rs"]
mod remedy_r3_tests;
