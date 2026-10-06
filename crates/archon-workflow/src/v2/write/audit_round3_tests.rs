//! Production write waves with takeover at capture and receipt boundaries.
use super::*;
use crate::{LifecycleAction, LifecycleController, RunStatus};
#[path = "../../../tests/support/write_wave_fixture.rs"]
mod support;
use crate::repository_audit::{
    budget::{AuditPolicy, Limit},
    runtime::AuditRuntime,
};
use support::{Edits, Fixture};

fn policy() -> AuditPolicy {
    AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited,
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    }
}
fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                result.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    result
}
fn edits() -> Vec<(Vec<&'static str>, Edits)> {
    vec![(
        vec!["owned.txt"],
        Edits {
            files: vec![("owned.txt", "landed\n")],
            report: vec!["owned.txt"],
            via_adapter: false,
        },
    )]
}
async fn gap(gap: &'static str, receipt_exists: bool, cancel: bool) {
    let f = Fixture::new();
    let mut run = f.store.load_state(&f.run).unwrap();
    run.status = RunStatus::Running;
    f.store.save_state(&run).unwrap();
    f.v2.bind_session_executor(run.generation);
    if gap == "cache_capture" {
        let out = f.wave("race", edits()).await;
        assert_eq!(out.status, WorkflowV2Status::Accepted);
    }
    let observed = Arc::new(std::sync::Mutex::new(None));
    let (store, id, observed_hook) = (f.store.clone(), f.run.clone(), observed.clone());
    super::audit_round3_hooks::install(
        f.v2.root().to_path_buf(),
        gap,
        Box::new(move || {
            let ctl = LifecycleController::new(store.clone());
            ctl.apply(&id, LifecycleAction::Pause).unwrap();
            ctl.apply(&id, LifecycleAction::Resume).unwrap();
            let successor = AuditRuntime::initialize(store.clone(), id.clone(), policy()).unwrap();
            successor
                .update(|state| {
                    state.attempts = 91;
                    state.last_error = Some("successor audit".into());
                    Ok(())
                })
                .unwrap();
            if cancel {
                ctl.apply(&id, LifecycleAction::Cancel).unwrap();
            }
            if receipt_exists {
                store
                    .write_run_file(
                        &id,
                        crate::repository_audit::receipts::ApplyReceipt::relative_path("race", 0),
                        b"successor receipt",
                    )
                    .unwrap();
            }
            *observed_hook.lock().unwrap() =
                Some(bytes(&store.run_dir(&id).join("v2/repository-audit")));
        }),
    );
    let (call, branches) = f.race_branches("race", edits());
    let out = f.race_wave(call, branches).await;
    assert!(out.is_err(), "stale wave must be refused: {out:?}");
    let before = observed
        .lock()
        .unwrap()
        .take()
        .expect("must reach named gap");
    assert!(
        bytes(&f.store.run_dir(&f.run).join("v2/repository-audit")) == before,
        "successor audit file contents and directory inventory must be untouched at {gap}"
    );
}
#[tokio::test]
async fn round3_291_cached_wave_capture_takeover() {
    gap("cache_capture", false, false).await;
}
#[tokio::test]
async fn round3_291_postapply_capture_takeover() {
    gap("apply_capture", false, false).await;
}
#[tokio::test]
async fn round3_291_apply_receipt_takeover() {
    gap("apply_receipt", false, false).await;
}
#[tokio::test]
async fn round3_291_apply_receipt_existing_successor_bytes() {
    gap("apply_receipt", true, false).await;
}
#[tokio::test]
async fn round3_291_apply_receipt_successor_cancelled() {
    gap("apply_receipt", false, true).await;
}

fn removed_run(action: LifecycleAction) {
    let f = Fixture::new();
    let mut run = f.store.load_state(&f.run).unwrap();
    run.status = RunStatus::Running;
    f.store.save_state(&run).unwrap();
    f.v2.bind_session_executor(run.generation);
    let audit = AuditRuntime::initialize(f.store.clone(), f.run.clone(), policy()).unwrap();
    audit.state().unwrap(); // Named state-read / capture gap.
    let ctl = LifecycleController::new(f.store.clone());
    ctl.apply(&f.run, LifecycleAction::Pause).unwrap();
    if action != LifecycleAction::Pause {
        ctl.apply(&f.run, action).unwrap();
    }
    std::fs::remove_dir_all(f.store.run_dir(&f.run)).unwrap();
    let names = || {
        std::fs::read_dir(f.store.root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>()
    };
    let before = names();
    let capture = audit.capture_snapshot(&f.repo, &["owned.txt".into()], &f.v2);
    // Creation is also a run-state writer: an executor-bound handle cannot
    // mint a foreign run before its ownership refusal.
    let creation = audit.store.create_run(run.spec.clone());
    assert!(capture.is_err() && creation.is_err());
    assert_eq!(
        names(),
        before,
        "a denied writer must create no run namespace"
    );
}
#[test]
fn round3_291_removed_paused_run_is_not_recreated() {
    removed_run(LifecycleAction::Pause);
}
#[test]
fn round3_291_removed_resumed_run_is_not_recreated() {
    removed_run(LifecycleAction::Resume);
}
#[test]
fn round3_291_removed_cancelled_run_is_not_recreated() {
    removed_run(LifecycleAction::Cancel);
}
