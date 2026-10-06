use super::*;
use crate::command::workflow_host_command_groups::{
    GROUP_RECORDS_DIR, record_group, require_no_running_groups,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct AccountingStall {
    child: std::sync::Mutex<std::process::Child>,
    pins: Vec<(u32, u64)>,
    reads: AtomicUsize,
    mode: u32,
}
impl Drop for AccountingStall {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl JobOps for AccountingStall {
    fn process_identities_observed(&self, progress: &Progress) -> io::Result<Vec<(u32, u64)>> {
        progress.check()?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        progress.advance();
        Ok(self.pins.clone())
    }
    fn kill_and_confirm_observed(&self, progress: &Progress) -> io::Result<u32> {
        let mut rounds = 0;
        archon_shell::teardown_progress::confirm_empty(progress, || {
            rounds += 1;
            Ok(if self.mode == 1 { rounds.min(2) } else { 1 })
        })
    }
}
async fn stall_collects_and_heals(mode: u32) {
    let run = tempfile::tempdir().unwrap();
    let child = archon_shell::spawn::command("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = archon_shell::process_tree::identity_of(pid)
        .unwrap()
        .unwrap();
    let job = Arc::new(AccountingStall {
        child: std::sync::Mutex::new(child),
        pins: vec![(pid, start)],
        reads: AtomicUsize::new(0),
        mode,
    });
    let mut ended = archon_shell::spawn::command("true").spawn().unwrap();
    let id = ended.id();
    ended.wait().unwrap();
    let guard = record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        id,
        id,
        None,
        None,
        "cmd",
    )
    .unwrap();
    if mode == 2 {
        std::fs::create_dir(guard.path().with_extension("json.tmp")).unwrap();
    }
    let outcome =
        kill_job_off_thread(job.clone(), guard.evidence(), Duration::from_millis(200)).await;
    let Teardown::Stalled { survivors, .. } = outcome else {
        panic!("accounting did not confirm empty")
    };
    assert_eq!(
        survivors.as_deref(),
        Some(job.pins.as_slice()),
        "stall discarded identities"
    );
    assert_eq!(
        job.reads.load(Ordering::SeqCst),
        2,
        "stall must actually enumerate again"
    );
    guard.keep(survivors.as_deref());
    assert!(require_no_running_groups(run.path(), "run").is_err());
    {
        let mut child = job.child.lock().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
    assert!(
        require_no_running_groups(run.path(), "run").is_ok(),
        "gone survivors should heal"
    );
}
#[tokio::test]
async fn unchanged_job_accounting_collects_identities_and_heals() {
    stall_collects_and_heals(0).await;
}
#[tokio::test]
async fn increasing_job_accounting_collects_identities_and_heals() {
    stall_collects_and_heals(1).await;
}
#[tokio::test]
async fn stall_collection_survives_record_rewrite_failure() {
    stall_collects_and_heals(2).await;
}
