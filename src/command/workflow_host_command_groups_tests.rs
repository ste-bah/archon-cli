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
    record_group(&dir, pgid, pgid, Some(pgid), None, "cmd")
        .unwrap()
        .keep(Some(&[pinned]));
    let record = read(&dir.join(format!("{pgid}.json")));
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
    record_group(&dir, pgid, pgid, Some(pgid), None, "cmd")
        .unwrap()
        .keep(None);
    let record = read(&dir.join(format!("{pgid}.json")));
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
    std::fs::create_dir(dir.join(format!("{pgid}.json.tmp"))).unwrap();
    guard.keep(Some(&[pinned]));
    let refused = require_no_running_groups(run.path(), "run");
    let _ = survivor.kill();
    let _ = survivor.wait();
    let refusal = refused.expect_err("an unwritten survivor list is not 'none left'");
    assert!(refusal.to_string().contains("unknown"), "{refusal}");
    assert!(dir.join(format!("{pgid}.unknown.json")).exists());
}

#[test]
fn both_survivor_rewrite_and_fallback_rename_failure_block_resume() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let pgid = ended_group();
    let guard = record_group(&dir, pgid, pgid, Some(pgid), None, "cmd").unwrap();
    std::fs::create_dir(dir.join(format!("{pgid}.json.tmp"))).unwrap();
    std::fs::create_dir(dir.join(format!("{pgid}.unknown.json"))).unwrap();
    guard.keep(None);
    // Restore both paths: a transient failure must leave persistent evidence.
    std::fs::remove_dir(dir.join(format!("{pgid}.json.tmp"))).unwrap();
    std::fs::remove_dir(dir.join(format!("{pgid}.unknown.json"))).unwrap();
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
    record_group(&dir, pgid, pgid, None, None, "cmd")
        .unwrap()
        .keep(Some(&[(42, 7)]));
    let record = read(&dir.join(format!("{pgid}.json")));
    let state = survivors_running(
        &record,
        |_| Err(std::io::ErrorKind::PermissionDenied.into()),
    );
    assert_eq!(state, None, "resume must refuse an unreadable identity");
}
