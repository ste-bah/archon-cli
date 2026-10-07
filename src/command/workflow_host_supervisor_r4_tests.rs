//! Real supervisor orchestration with injected kernel/checkpoint faults.
use super::termination::{Tree, job::JobOps};
use super::*;
use crate::command::workflow_host_command_groups::{
    GROUP_RECORDS_DIR, require_no_running_groups, stalled_running,
};
use archon_shell::{
    process_tree::{Pinned, deliver, identity_of},
    teardown_progress::Progress,
};
use std::{cell::RefCell, io, path::Path, sync::Arc};
thread_local! {
    static BEFORE: RefCell<Option<Box<dyn FnOnce(&mut Tree)>>> = RefCell::new(None);
}
pub(super) fn before_teardown(tree: &mut Tree) {
    if let Some(hook) = BEFORE.with(|hook| hook.borrow_mut().take()) {
        hook(tree);
    }
}
fn request(root: &Path, mode: u32, pipe: u32) -> ResolvedHostCommand {
    // The real descendant owns inherited pipes, ignores TERM and overwrites a
    // bounded heartbeat file. The leader waits until it really started.
    let child = format!(
        "import os,signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); open('ready.tmp','w').write(str(os.getpid())); os.rename('ready.tmp','ready'); {}\nwhile True:\n open('heartbeat','w').write(str(time.monotonic_ns())); time.sleep(.02)",
        match pipe {
            0 => "os.close(2)",
            1 => "os.close(1)",
            _ => "pass",
        }
    );
    let script = format!(
        "import subprocess,time,os; subprocess.Popen(['python3','-c',{}]);\nwhile not os.path.exists('ready'): time.sleep(.01)\n{}",
        serde_json::to_string(&child).unwrap(),
        match mode {
            0 => "pass",
            1 => "time.sleep(30)",
            _ => "os.write(1,b'x'*4096); time.sleep(30)",
        }
    );
    ResolvedHostCommand {
        command_id: "fixture".into(),
        program: "python3".into(),
        args: vec!["-c".into(), script],
        cwd: root.into(),
        environment: super::super::workflow_host_environment::process_environment(),
        stdin: None,
        timeout_secs: if mode == 1 { 1 } else { 10 },
        max_stdout_bytes: if mode == 2 { 256 } else { 4096 },
        max_stderr_bytes: 4096,
        declared_write_set: vec![],
        remediation_scopes: Default::default(),
    }
}
async fn stop(pin: Pinned) {
    deliver(pin, libc::SIGKILL);
    let began = std::time::Instant::now();
    loop {
        let gone = match identity_of(pin.pid) {
            Ok(Some(start)) => start != pin.start,
            Ok(None) => true,
            Err(_) => false, // a transient unreadable identity is not an exit
        };
        if gone {
            break;
        }
        deliver(pin, libc::SIGKILL);
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "fixture survivor did not exit"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn marker(records: &Path) -> std::path::PathBuf {
    std::fs::read_dir(records)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "pending"))
        .unwrap()
}
async fn checkpoint_failure(mode: u32) {
    let run = tempfile::tempdir().unwrap();
    let records = run.path().join(GROUP_RECORDS_DIR);
    let marker_dir = records.clone();
    let ready = run.path().join("ready");
    let owned = Arc::new(std::sync::Mutex::new(None));
    let captured = owned.clone();
    BEFORE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |tree| {
            let pid = std::fs::read_to_string(ready).unwrap().parse().unwrap();
            let start = identity_of(pid).unwrap().unwrap();
            *captured.lock().unwrap() = Some(Pinned { pid, start });
            tree.evidence
                .as_ref()
                .unwrap()
                .fail_writes(&marker(&marker_dir));
        }))
    });
    let (control, _handle) = HostCommandControl::new();
    let result =
        supervise_process_group(request(run.path(), mode, 2), control, Some(&records)).await;
    let pin = owned.lock().unwrap().unwrap();
    let reading = std::time::Instant::now();
    let alive = loop {
        assert!(
            reading.elapsed() < Duration::from_secs(5),
            "fixture identity remained unreadable"
        );
        match identity_of(pin.pid) {
            Ok(value) => break value == Some(pin.start),
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    };
    stop(pin).await;
    assert!(
        !alive,
        "checkpoint failure skipped descendant termination ({mode})"
    );
    assert!(
        result.is_ok(),
        "checkpoint failure must be operational: {result:?}"
    );
    let output = result.unwrap();
    assert!(output.timed_out || output.exit_code == Some(75));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("recording teardown identities failed")
    );
    assert!(
        require_no_running_groups(run.path(), "run").is_err(),
        "failed checkpoint falsely healed"
    );
    // Once the real descendant is proven gone, apply the exact manual remedy
    // for incomplete checkpoint evidence and exercise healing on the same run.
    for entry in std::fs::read_dir(&records).unwrap().flatten() {
        std::fs::remove_file(entry.path()).unwrap();
    }
    assert!(require_no_running_groups(run.path(), "run").is_ok());
}
#[tokio::test]
async fn completed_supervisor_terminates_despite_checkpoint_failure() {
    checkpoint_failure(0).await;
}
#[tokio::test]
async fn timeout_supervisor_terminates_despite_checkpoint_failure() {
    checkpoint_failure(1).await;
}
#[tokio::test]
async fn overflow_supervisor_terminates_despite_checkpoint_failure() {
    checkpoint_failure(2).await;
}

struct StalledJob {
    pin: Pinned,
    evidence: Option<super::super::workflow_host_command_groups::GroupEvidence>,
    reads: std::sync::atomic::AtomicUsize,
    fault: u32,
    marker: std::path::PathBuf,
}
impl JobOps for StalledJob {
    fn process_identities_observed(&self, progress: &Progress) -> io::Result<Vec<(u32, u64)>> {
        progress.check()?;
        if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0
            && let Some(evidence) = &self.evidence
        {
            if self.fault == 3 {
                drop(evidence.contend(Duration::from_secs(3)));
            } else {
                evidence.fail_writes(&self.marker);
            }
        }
        progress.advance();
        Ok(vec![(self.pin.pid, self.pin.start)])
    }
    fn kill_and_confirm_observed(&self, progress: &Progress) -> io::Result<u32> {
        // Model a kernel that accepts job termination but still reports active
        // processes. Exercise the production job collector and supervisor.
        deliver(self.pin, libc::SIGTERM);
        progress.check()?;
        Ok(1)
    }
}
async fn known_survivor_fault(pipe: u32, fault: u32) {
    let run = tempfile::tempdir().unwrap();
    let ready = run.path().join("ready");
    BEFORE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |tree| {
            let pid = std::fs::read_to_string(&ready).unwrap().parse().unwrap();
            let start = identity_of(pid).unwrap().unwrap();
            let pending = marker(&ready.parent().unwrap().join(GROUP_RECORDS_DIR));
            if fault == 1 {
                tree.evidence.as_ref().unwrap().fail_writes(&pending);
            }
            tree.test_job = Some(Arc::new(StalledJob {
                pin: Pinned { pid, start },
                reads: Default::default(),
                fault,
                marker: pending,
                evidence: if fault >= 2 {
                    tree.evidence.clone()
                } else {
                    None
                },
            }));
        }))
    });
    let (control, _handle) = HostCommandControl::new();
    let result = supervise_process_group(
        request(run.path(), 0, pipe),
        control,
        Some(&run.path().join(GROUP_RECORDS_DIR)),
    )
    .await;
    let pid = std::fs::read_to_string(run.path().join("ready"))
        .unwrap()
        .parse()
        .unwrap();
    let start = identity_of(pid).unwrap().unwrap();
    let before = stalled_running(run.path()).unwrap();
    let refused = require_no_running_groups(run.path(), "run");
    stop(Pinned { pid, start }).await;
    let healed = require_no_running_groups(run.path(), "run");
    assert!(result.is_ok(), "{result:?}");
    let output = result.unwrap();
    assert_eq!(output.exit_code, Some(75));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("pipes stayed open"),
        "drain timeout not exercised"
    );
    assert!(refused.is_err(), "living survivor falsely healed");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].survivors, vec![(pid, start)]);
    assert!(
        !before[0].survivors_unknown,
        "pipe diagnostics erased established identities"
    );
    assert!(
        healed.is_ok(),
        "known exited survivors did not heal: {healed:?}"
    );
    assert!(stalled_running(run.path()).unwrap().is_empty());
}
#[tokio::test]
async fn stdout_survivor_survives_terminate_drain_settle_and_heals() {
    known_survivor_fault(0, 0).await;
}
#[tokio::test]
async fn stderr_survivor_survives_terminate_drain_settle_and_heals() {
    known_survivor_fault(1, 0).await;
}
#[tokio::test]
async fn both_pipe_survivor_survives_terminate_drain_settle_and_heals() {
    known_survivor_fault(2, 0).await;
}

#[tokio::test]
async fn fresh_job_identities_survive_failed_preparation_and_heal() {
    known_survivor_fault(0, 1).await;
}
#[tokio::test]
async fn fresh_job_identities_survive_failed_completion_checkpoint_and_heal() {
    known_survivor_fault(1, 2).await;
}

#[tokio::test]
async fn fresh_job_identities_survive_checkpoint_contention_and_heal() {
    known_survivor_fault(2, 3).await;
}
