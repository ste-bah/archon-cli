//! Round 9 (#297): a call's staging carries a durable residue record from
//! before its child runs until it is sealed after a confirmed teardown; the
//! exit drains pending teardown and sealing work on every platform; and a
//! dropped supervisor keeps its group record until the deferred seal ran.
use super::*;
use crate::command::workflow_host_command_groups::{GROUP_RECORDS_DIR, require_no_running_groups};
use crate::command::workflow_host_command_supervisor::supervise_process_group;
use crate::command::workflow_host_command_teardown_latch::TeardownLatch;
use crate::command::workflow_host_exit_drain::{CAP, NO_PROGRESS, drain};
use crate::command::workflow_host_staging_residue::{RESIDUE_DIR, clear_left};

fn residue_record(fixture: &Fixture) -> PathBuf {
    let name = fixture.call_root.file_name().unwrap().to_string_lossy();
    fixture
        .store
        .run_dir(&fixture.run_id)
        .join(RESIDUE_DIR)
        .join(format!("{name}.json"))
}

/// The teardown thread never reports (it hangs, or the process dies with
/// it): no seal runs and no event is written, but the record written before
/// the child ran names the staging, and the next resume removes it.
#[tokio::test]
async fn a_teardown_that_never_reports_leaves_a_record_the_resume_clears() {
    let (fixture, process) = cancelled(Stop::Owned).await;
    let token = process.token.lock().unwrap().take().expect("tree tracked");
    std::mem::forget(token);
    assert!(
        fixture.call_root.join("late.txt").exists(),
        "nothing sealed"
    );
    assert!(events_named(&fixture, "host_command_staging_residue").is_empty());
    let record = residue_record(&fixture);
    assert!(record.exists(), "no durable record names the staging");
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let cleared = clear_left(&run_dir).unwrap();
    assert_eq!(cleared, vec![fixture.call_root.clone()]);
    assert!(
        !fixture.call_root.exists(),
        "the secret-bearing staging stayed"
    );
    assert!(!record.exists());
    assert!(clear_left(&run_dir).unwrap().is_empty(), "cleared once");
}

#[tokio::test]
async fn the_record_goes_only_after_a_seal_that_follows_a_confirmed_teardown() {
    let (fixture, process) = cancelled(Stop::Owned).await;
    assert!(
        residue_record(&fixture).exists(),
        "kept while the seal waits"
    );
    teardown(&process, true);
    no_clear_secret(&fixture);
    assert!(!residue_record(&fixture).exists(), "a sealed call keeps it");
    let (fixture, process) = cancelled(Stop::Owned).await;
    teardown(&process, false);
    assert!(
        residue_record(&fixture).exists(),
        "an unconfirmed teardown may leave a writer: the record stays"
    );
    std::fs::write(fixture.call_root.join("survivor.txt"), CANARY).unwrap();
    clear_left(&fixture.store.run_dir(&fixture.run_id)).unwrap();
    assert!(!fixture.call_root.exists());
}

#[tokio::test]
async fn a_completed_call_leaves_no_residue_record() {
    let fixture = fixture(Arc::new(SecretPrintingProcess(Child::Clean)));
    let result = fixture
        .executor
        .execute(request(), Some(fixture.generation))
        .await
        .unwrap();
    assert!(result.publication_receipt.is_some(), "{result:?}");
    assert!(!residue_record(&fixture).exists());
}

/// The resume never removes staging while a recorded group may still run.
#[tokio::test]
async fn the_resume_keeps_the_staging_while_a_recorded_group_runs() {
    let (fixture, process) = cancelled(Stop::Owned).await;
    std::mem::forget(process.token.lock().unwrap().take());
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let me = std::process::id();
    let guard = crate::command::workflow_host_command_groups::record_group(
        &run_dir.join(GROUP_RECORDS_DIR),
        me,
        me,
        None,
        None,
        "fixture",
    )
    .unwrap();
    assert!(clear_left(&run_dir).unwrap().is_empty());
    assert!(fixture.call_root.exists() && residue_record(&fixture).exists());
    drop(guard);
    assert_eq!(
        clear_left(&run_dir).unwrap(),
        vec![fixture.call_root.clone()]
    );
}

/// A sealing failure after the commit: the run pauses, and the call still
/// returns its receipt, with the residue recorded.
#[tokio::test]
async fn a_seal_failure_after_the_commit_pauses_the_run_and_keeps_the_receipt() {
    let fixture = fixture(Arc::new(SecretPrintingProcess(Child::Clean)));
    crate::command::workflow_host_command_exec::AFTER_CALL.set(Some(Box::new(|root| {
        let locked = root.join("locked");
        std::fs::create_dir(&locked).unwrap();
        set_mode(&locked, 0o000);
    })));
    let result = fixture
        .executor
        .execute(request(), Some(fixture.generation))
        .await;
    let result = result.expect("a committed publication keeps its result");
    assert!(result.publication_receipt.is_some(), "{result:?}");
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused, "the sealing failure pauses");
    let pauses = events_named(&fixture, "host_command_staging_pause");
    assert_eq!(pauses.len(), 1, "{pauses:?}");
    let residue = events_named(&fixture, "host_command_staging_residue");
    assert_eq!(residue.len(), 1, "{residue:?}");
    assert!(
        !fixture.call_root.exists(),
        "the unsealed staging was removed"
    );
}

fn trapping_child(dir: &Path) -> ResolvedHostCommand {
    ResolvedHostCommand {
        command_id: "fixture".into(),
        program: "sh".into(),
        args: vec![
            "-c".into(),
            "trap '' TERM; echo $$ > pid.tmp; mv pid.tmp pid; exec sleep 30".into(),
        ],
        cwd: dir.into(),
        environment: crate::command::workflow_host_environment::process_environment(),
        stdin: None,
        timeout_secs: 60,
        max_stdout_bytes: 4096,
        max_stderr_bytes: 4096,
        declared_write_set: vec![],
        remediation_scopes: Default::default(),
        spill_dir: None,
    }
}

/// Starts the child under the real supervisor and drops the supervisor once
/// the child runs, as a cancelled call does.
async fn dropped_supervisor(dir: &Path, records: Option<&Path>) -> TeardownLatch {
    let latch = TeardownLatch::default();
    let (control, _handle) = HostCommandControl::tracked(latch.clone());
    let mut work = Box::pin(supervise_process_group(
        trapping_child(dir),
        control,
        records,
    ));
    let began = std::time::Instant::now();
    while !dir.join("pid").exists() {
        assert!(began.elapsed() < std::time::Duration::from_secs(10));
        let tick = tokio::time::timeout(std::time::Duration::from_millis(20), &mut work).await;
        assert!(tick.is_err(), "the child ended early: {tick:?}");
    }
    assert!(latch.pending(), "a running tree is tracked");
    drop(work);
    latch
}

/// The process exit waits for a dropped supervisor's teardown and for the
/// sealing that waits on it, on every platform.
#[tokio::test]
async fn the_exit_drain_waits_for_a_dropped_supervisors_teardown_and_seal() {
    let dir = tempfile::tempdir().unwrap();
    let latch = dropped_supervisor(dir.path(), None).await;
    let sealed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = sealed.clone();
    latch.after_teardown(Box::new(move |_| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        flag.store(true, Ordering::SeqCst);
    }));
    let drained = tokio::task::spawn_blocking(|| drain(NO_PROGRESS, CAP))
        .await
        .unwrap();
    assert!(drained, "the drain gave up on a teardown that progresses");
    assert!(
        sealed.load(Ordering::SeqCst),
        "the exit would skip the seal"
    );
}

/// A resume cannot start while the cancelled call's deferred seal runs: the
/// group record stays until that seal has returned.
#[tokio::test]
async fn a_dropped_supervisor_keeps_its_group_record_until_the_deferred_seal_ran() {
    let run_dir = tempfile::tempdir().unwrap();
    let records = run_dir.path().join(GROUP_RECORDS_DIR);
    let work_dir = tempfile::tempdir().unwrap();
    let latch = dropped_supervisor(work_dir.path(), Some(&records)).await;
    let (sender, receiver) = std::sync::mpsc::channel();
    let probed = run_dir.path().to_path_buf();
    latch.after_teardown(Box::new(move |confirmed| {
        let resume = require_no_running_groups(&probed, "run").map(drop);
        sender
            .send((confirmed, resume.map_err(|error| error.to_string())))
            .unwrap();
    }));
    let (confirmed, resume) = tokio::task::spawn_blocking(move || {
        receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the teardown reported")
    })
    .await
    .unwrap();
    assert!(confirmed, "the teardown confirmed the tree empty");
    assert!(resume.is_err(), "a resume could start during the seal");
    let drained = tokio::task::spawn_blocking(|| drain(NO_PROGRESS, CAP));
    assert!(drained.await.unwrap());
    require_no_running_groups(run_dir.path(), "run").expect("the record went after the seal");
}
