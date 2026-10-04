//! Issue 270 round 3: what a kept record says about a stalled teardown, and
//! that it never reads as "nothing left" when that cannot be shown.
use super::*;

/// A pgid no live process carries, as after the command's leader exited.
fn ended_group() -> u32 {
    let child = std::process::Command::new("true").spawn().unwrap();
    let id = child.id();
    drop(child.wait_with_output());
    id
}

fn sleeper() -> (std::process::Child, (u32, u64)) {
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = archon_shell::process_tree::start_of(pid).unwrap();
    (child, (pid, start))
}

fn read(path: &Path) -> HostCommandGroupRecord {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_kept_record_names_its_survivors_and_runs_while_they_live() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let (mut survivor, pinned) = sleeper();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    guard.keep(Some(&[pinned]));
    let record = read(&path);
    assert!(record.stalled && !record.survivors_unknown);
    assert_eq!(record.survivors, vec![pinned]);
    let refusal = require_no_running_groups(run.path(), "run").unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains(&format!("survivors: {}", pinned.0)),
        "{refusal}"
    );
    assert_eq!(stalled_running(run.path()).unwrap().len(), 1);
    let _ = survivor.kill();
    let _ = survivor.wait();
    // The survivor gone and its group with it, the record ends.
    let start = std::time::Instant::now();
    while require_no_running_groups(run.path(), "run").is_err() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_record_with_unknown_survivors_runs_until_someone_verifies_the_tree() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    guard.keep(None);
    let record = read(&path);
    assert!(record.stalled && record.survivors_unknown);
    assert_eq!(record_running(&record), None);
    let refusal = require_no_running_groups(run.path(), "run").unwrap_err();
    assert!(refusal.to_string().contains("unknown"), "{refusal}");
}

#[test]
fn a_survivor_list_that_cannot_be_written_never_leaves_the_old_empty_list() {
    // The staging write fails (here: the staging name is a directory, as a
    // full disk would fail it). The old contents (no survivors) would let a
    // resume go ahead once the group ended; the record must instead mean
    // "unknown survivors".
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let (mut survivor, pinned) = sleeper();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
    guard.keep(Some(&[pinned]));
    let refused = require_no_running_groups(run.path(), "run");
    let _ = survivor.kill();
    let _ = survivor.wait();
    let refusal = refused.expect_err("an unwritten survivor list is not 'none left'");
    assert!(refusal.to_string().contains("unknown"), "{refusal}");
    assert!(unknown_path(&path).exists());
}

#[test]
fn both_survivor_rewrite_and_fallback_rename_failure_block_resume() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
    std::fs::create_dir(unknown_path(&path)).unwrap();
    guard.keep(None);
    // Restore both paths: a transient failure must leave persistent evidence.
    std::fs::remove_dir(path.with_extension("json.tmp")).unwrap();
    std::fs::remove_dir(unknown_path(&path)).unwrap();
    assert!(
        require_no_running_groups(run.path(), "run").is_err(),
        "resume forgot the unknown survivor"
    );
    assert_eq!(stalled_running(run.path()).unwrap().len(), 1);
}

#[test]
fn an_unreadable_retained_survivor_cannot_be_treated_as_dead() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let guard = record_group(&dir, pgid, pgid, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    guard.keep(Some(&[(42, 7)]));
    let record = read(&path);
    let state = survivors_running(
        &record,
        |_| Err(std::io::ErrorKind::PermissionDenied.into()),
    );
    assert_eq!(state, None, "resume must refuse an unreadable identity");
}

/// A live process-group leader that is not any command's: a stranger.
fn group_leader() -> (std::process::Child, (u32, u64)) {
    use std::os::unix::process::CommandExt;
    let child = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = archon_shell::process_tree::start_of(pid).unwrap();
    (child, (pid, start))
}

/// Every record file under `dir`, rewritten as if written by `host_pid`.
fn owned_by(dir: &Path, host_pid: u32) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["host_pid"] = host_pid.into();
        std::fs::write(&path, value.to_string()).unwrap();
    }
}

#[test]
fn a_live_sibling_is_not_a_stall_but_a_kept_one_is() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let (mut live, (live_pgid, _)) = group_leader();
    let (mut kept, (kept_pgid, kept_start)) = group_leader();
    let running = record_group(&dir, live_pgid, live_pgid, None, None, "running").unwrap();
    record_group(&dir, kept_pgid, kept_pgid, None, None, "stalled")
        .unwrap()
        .keep(Some(&[(kept_pgid, kept_start)]));
    let stalled = stalled_running(run.path()).unwrap();
    drop(running);
    for child in [&mut live, &mut kept] {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert_eq!(
        stalled
            .iter()
            .map(|r| r.command_id.as_str())
            .collect::<Vec<_>>(),
        vec!["stalled"],
        "only the kept record is a stalled teardown"
    );
}

#[test]
fn a_crashed_owners_record_ends_with_its_group_and_session() {
    // The executor died mid-command: no teardown will ever settle its
    // record. Once its owner is gone and its group and session have ended,
    // the record ends too; it does not wait for someone to remove it.
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    std::mem::forget(record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap());
    owned_by(&dir, ended_group());
    let resumed = require_no_running_groups(run.path(), "run");
    assert!(resumed.is_ok(), "{resumed:?}");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "the record stayed"
    );
}

#[test]
fn a_crashed_owners_record_runs_while_its_group_does() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let (mut leader, (pgid, _)) = group_leader();
    std::mem::forget(record_group(&dir, pgid, pgid, None, None, "cmd").unwrap());
    owned_by(&dir, ended_group());
    let refused = require_no_running_groups(run.path(), "run");
    let _ = leader.kill();
    let _ = leader.wait();
    assert!(
        refused.is_err(),
        "a live group of a crashed owner was let go"
    );
}

#[test]
fn a_stale_marker_never_blocks_a_new_command_with_the_same_group_id() {
    // A group id is reused once the group ended. A record left by the old
    // command must neither block the new one nor be overwritten by it.
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    std::mem::forget(record_group(&dir, pgid, pgid, Some(pgid), None, "old").unwrap());
    let before = std::fs::read_dir(&dir).unwrap().count();
    let new = record_group(&dir, pgid, pgid, Some(pgid), None, "new");
    let after = std::fs::read_dir(&dir).unwrap().count();
    assert!(new.is_ok(), "{:?}", new.err());
    assert_eq!(after, before * 2, "the old record was overwritten");
}

#[test]
fn a_group_id_reused_by_a_stranger_is_not_the_recorded_group() {
    // The recorded leader is gone and a stranger now leads a group of the
    // same number: the kernel never reuses a pid while a group of that id
    // exists, so the recorded group has ended.
    let (mut stranger, (pgid, start)) = group_leader();
    let record = |leader_start: u64| -> HostCommandGroupRecord {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "pgid": pgid,
            "pid": pgid,
            "leader_start": leader_start,
            "command_id": "cmd",
            "host_pid": 1,
            "started_at": "2026-10-04T00:00:00Z",
        }))
        .unwrap()
    };
    let (reused, ours) = (
        record_running(&record(start + 1)),
        record_running(&record(start)),
    );
    let _ = stranger.kill();
    let _ = stranger.wait();
    assert_eq!(reused, Some(false), "a stranger's group was taken for ours");
    assert_eq!(ours, Some(true), "the recorded leader still runs");
}

#[test]
fn an_unwritten_stall_stays_unknown_after_its_owner_exits() {
    // Guard: unlike a crash, a teardown that stalled and could not write its
    // survivors leaves only its marker, which needs someone to verify.
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    let blocked = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .flat_map(|path| [path.with_extension("json.tmp"), unknown_path(&path)])
            .collect()
    };
    let blockers = blocked(&dir);
    for path in &blockers {
        std::fs::create_dir(path).unwrap();
    }
    guard.keep(None);
    for path in &blockers {
        std::fs::remove_dir(path).unwrap();
    }
    owned_by(&dir, ended_group());
    let refusal = require_no_running_groups(run.path(), "run").unwrap_err();
    assert!(refusal.to_string().contains("unknown"), "{refusal}");
}
