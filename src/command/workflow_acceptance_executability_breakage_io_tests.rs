use super::*;
#[tokio::test]
async fn private_evidence_redacted_and_store_failure_recorded() {
    let secret = "breakage-secret-7b8c9d0e1f2a";
    crate::command::workflow_task_set::passability::evidence::test_secret("SERVICE_TOKEN", secret);
    let f = Fixture::new(&format!("printf '{secret}'; test \"$(cat value)\" = good"));
    let r = f.prove(&f.probe(true), 10).await;
    for run in r.proofs.runs.values() {
        let bytes = std::fs::read(
            r.store
                .as_ref()
                .unwrap()
                .join(run.evidence.output.as_ref().unwrap()),
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(
                    r.store
                        .as_ref()
                        .unwrap()
                        .join(run.evidence.output.as_ref().unwrap())
                )
                .unwrap()
                .permissions()
                .mode()
                    & 0o777,
                0o600
            );
        }
    }
    let f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    let cache = f
        .source
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR);
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, b"blocked").unwrap();
    let r = f.prove(&p, 10).await;
    assert!(
        r.proofs
            .summary
            .as_ref()
            .unwrap()
            .refusals
            .iter()
            .any(|s| s == "proofs_not_saved")
    );
    assert!(r.proofs.runs.values().all(|r| r.counted));
}
#[tokio::test]
async fn changed_check_and_corrupted_cache_rerun() {
    let mut f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    let first = f.prove(&p, 10).await;
    let runs = first.store.unwrap().join("runs");
    let corrupt = std::fs::read_dir(&runs)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(corrupt, b"invalid").unwrap();
    let second = f.prove(&p, 10).await;
    let s = second.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (3, 1));
    assert!(s.refusals.contains(&"proofs_unreadable".into()));
    if let AcceptanceCheck::Command { command, .. } = &mut f.contract.acceptance[0].check {
        command.push_str("; test -f value");
    }
    f.lock
        .baseline_results
        .get_mut("check")
        .unwrap()
        .check_digest = check_digest(&f.contract.acceptance[0].check);
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    let r = f.prove(&p, 10).await;
    let s = r.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (4, 0));
}
#[tokio::test]
async fn base_output_is_private_and_regular_copy_stays_warm() {
    let f = Fixture::new("test -f value && env");
    let p = f.probe(false);
    let base = Arc::new(f.copy());
    let req = BreakageRequest {
        contract: &f.contract,
        chain_digest: &f.lock.digest,
        lock: &f.lock,
        classification: &f.classes,
        mutants: &f.mutants,
        max_runs: 10,
        min_deadline_secs: 1,
    };
    let (outcome, output, _) = run::observe(&p, &req, &base, "check", &[], None).await;
    assert!(
        matches!(outcome, RunOutcome::Survived { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        String::from_utf8(output.unwrap())
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("CARGO_TARGET_DIR="))
            .unwrap(),
        f.parent.path().join("output").display().to_string()
    );
    let copy = HermeticCopy::at(
        f.parent.path(),
        f.source.path(),
        f.source.path(),
        None,
        f.lock.baseline_commit.as_ref().unwrap(),
    )
    .unwrap();
    let refs = refs_for(&f.contract, &f.lock.digest, &["check".into()].into());
    let run = run_refs_in_copy(
        &p,
        &copy,
        f.source.path(),
        &f.contract,
        &f.lock.digest,
        &refs,
        &archon_workflow::acceptance_scratch::ObserveHooks::for_resource_limits(
            p.resource_limits.clone(),
        ),
        &Mutex::new(None),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(run.results[0].stdout.clone())
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("CARGO_TARGET_DIR="))
            .unwrap(),
        warm_target(f.parent.path(), f.source.path())
            .display()
            .to_string()
    );
}
#[test]
fn fetch_exit_typing_and_materialise_failure() {
    let f = Fixture::new("true");
    let unknown = base_copy::baseline_available(f.source.path(), &"0".repeat(40)).unwrap_err();
    assert_eq!(
        unknown.2,
        Some(crate::command::workflow_host_probe_fault::UnrunCause::BaselineUnavailable)
    );
    let transport = f.parent.path().join("broken");
    std::fs::write(&transport, b"not a tree").unwrap();
    let failure = materialise_base_copy(
        f.parent.path(),
        f.source.path(),
        f.source.path(),
        None,
        f.snapshot(),
        f.lock.baseline_commit.as_ref().unwrap(),
        &f.parent.path().join("output"),
        Some(&transport),
    );
    assert!(matches!(
        failure,
        Err(Unrun(
            _,
            _,
            Some(crate::command::workflow_host_probe_fault::UnrunCause::BaselineFetchFailed)
        ))
    ));
    let mut snapshot = f.snapshot().clone();
    snapshot.digest = "invalid".into();
    let failure = materialise_base_copy(
        f.parent.path(),
        f.source.path(),
        f.source.path(),
        None,
        &snapshot,
        f.lock.baseline_commit.as_ref().unwrap(),
        &f.parent.path().join("output"),
        None,
    );
    assert!(matches!(
        failure,
        Err(Unrun(
            _,
            _,
            Some(crate::command::workflow_host_probe_fault::UnrunCause::SnapshotTampered)
        ))
    ));
    for outcome in [
        RunOutcome::BudgetExhausted,
        RunOutcome::NoExitStatus,
        decide::unrun("host_io", "failed"),
    ] {
        assert!(!store::reusable(&outcome));
    }
}
#[test]
fn absent_program_is_a_host_error() {
    const CASE: &str = "BREAKAGE_ABSENT_PROGRAM";
    if let Some(root) = std::env::var_os(CASE) {
        let error = base_copy::baseline_available(Path::new(&root), "missing").unwrap_err();
        assert_eq!(error.2, None);
        let isolated = base_git::IsolatedGit::new(Path::new(&root)).unwrap();
        assert_eq!(isolated.output(&["version".as_ref()]).unwrap_err().2, None);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","command::workflow_task_set::executability::hermetic::breakage::tests::io_cases::absent_program_is_a_host_error","--nocapture"]).env(CASE,root.path()).env("PATH",empty.path()).status().unwrap();
    assert!(status.success());
}
#[tokio::test]
async fn fresh_unstable_control_voids_reused_kills() {
    let marker = tempfile::tempdir().unwrap();
    let path = marker.path().join("marker");
    let command = format!(
        "if test -f '{}'; then exit 1; fi; if test \"$(cat value)\" = nope; then touch '{}'; exit 1; fi; test \"$(cat value)\" = good",
        path.display(),
        path.display()
    );
    let mut f = Fixture::new(&command);
    let p = f.probe(true);
    assert!(
        f.prove(&p, 10)
            .await
            .proofs
            .runs
            .values()
            .all(|r| r.counted)
    );
    f.mutants.mutants[0].patch = f.mutants.mutants[0].patch.replace("evil", "nope");
    let r = f.prove(&p, 10).await;
    assert_eq!(r.proofs.summary.as_ref().unwrap().runs_reused, 1);
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| !r.counted && r.not_counted == Some(NotCounted::ControlUnstable))
    );
    std::fs::remove_file(path).unwrap();
    let r = f.prove(&p, 0).await;
    assert_eq!(r.proofs.summary.unwrap().runs_reused, 0);
}
#[tokio::test]
async fn identical_patch_for_two_pairs_executes_once() {
    let mut f = Fixture::new("test \"$(cat value)\" = good");
    f.mutants.mutants.truncate(1);
    f.contract.acceptance[0].covers.push("second".into());
    let mut entry = f.classes.entries[0].clone();
    entry.id = "second".into();
    f.classes.entries.push(entry);
    let mut mutant = f.mutants.mutants[0].clone();
    mutant.requirement_id = "second".into();
    f.mutants.mutants.push(mutant);
    let p = f.probe(true);
    let first = f.prove(&p, 10).await;
    let summary = first.proofs.summary.unwrap();
    assert_eq!((summary.runs_executed, summary.runs_reused), (3, 1));
    assert_eq!(first.proofs.runs.len(), 2);
    assert!(first.proofs.runs.values().all(|r| r.counted));
    assert_eq!(p.resume.progress.reused_count(), 1);
    let keys: Vec<_> = std::fs::read_dir(first.store.unwrap().join("runs"))
        .unwrap()
        .collect();
    assert_eq!(keys.len(), 1);
    let second = f.prove(&p, 0).await;
    let summary = second.proofs.summary.unwrap();
    assert_eq!((summary.runs_executed, summary.runs_reused), (0, 2));
}
#[tokio::test]
async fn materialise_and_patch_write_errors_leave_other_work_running() {
    use crate::command::workflow_host_fault_test_hooks::{Point, inject};
    for failure in ["transport", "patches"] {
        let mut f = Fixture::new("test \"$(cat value)\" = good");
        if failure == "transport" {
            let mut check = f.contract.acceptance[0].clone();
            check.id = "zz-check".into();
            f.contract.acceptance.push(check);
            f.classes.entries[0]
                .checks
                .insert("zz-check".into(), Some(BaselineOutcome::Passed));
            let baseline = f.lock.baseline_results["check"].clone();
            f.lock.baseline_results.insert("zz-check".into(), baseline);
            f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
        }
        let parent = f.parent.path().to_path_buf();
        let hook = inject(
            Point::Direct,
            f.source.path(),
            Box::new(move || {
                let work = std::fs::read_dir(&parent)?
                    .find_map(|entry| {
                        let e = entry.ok()?;
                        if !e.file_type().ok()?.is_dir() {
                            return None;
                        }
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{COPY_PREFIX}breakage-"))
                            .then(|| e.path())
                    })
                    .expect("engine work root");
                assert!(work.join("build-output").is_dir());
                let path = work.join(if failure == "transport" {
                    "base.git"
                } else {
                    "patches"
                });
                std::fs::remove_dir_all(&path)?;
                if failure == "patches" {
                    std::fs::write(path, b"blocked")?;
                }
                Ok(())
            }),
        );
        let r = f.prove(&f.probe(false), 20).await;
        assert!(hook.fired());
        if failure == "transport" {
            assert_eq!(r.proofs.summary.unwrap().runs_executed, 4);
            assert!(
                r.proofs
                    .runs
                    .values()
                    .filter(|r| r.check_id == "check")
                    .all(|r| r.counted)
            );
            assert!(r.proofs.runs.values().filter(|r|r.check_id=="zz-check").all(|r|matches!(&r.outcome,RunOutcome::Unrun{code,..} if code=="baseline_fetch_failed")));
        } else {
            assert_eq!(
                r.proofs.summary.as_ref().unwrap().runs_executed,
                2,
                "{:?}",
                r.proofs
            );
            assert!(
                r.proofs
                    .runs
                    .values()
                    .all(|r| matches!(&r.outcome,RunOutcome::Unrun{code,..} if code=="host_io"))
            );
            assert!(r.proofs.controls.values().all(|c| c.stable()));
        }
    }
}
#[tokio::test]
async fn invalid_cache_fields_and_transient_outcomes_are_not_reused() {
    let f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    let first = f.prove(&p, 10).await;
    let root = first.store.unwrap();
    let path = std::fs::read_dir(root.join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    for changed in ["schema", "key", "outcome", "zero_kill", "control", "check"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match changed {
            "schema" => value["schema"] = serde_json::json!(99),
            "key" => value["key"] = serde_json::json!("wrong"),
            "outcome" => value["outcome"] = serde_json::json!({"outcome":"budget_exhausted"}),
            "zero_kill" => {
                value["outcome"] =
                    serde_json::json!({"outcome":"killed","exit_code":0,"elapsed_ms":0})
            }
            "control" => value["control"]["post"] = serde_json::json!({"outcome":"no_exit_status"}),
            _ => value["control"]["check_digest"] = serde_json::json!("wrong"),
        };
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let r = f.prove(&p, 10).await;
        let summary = r.proofs.summary.unwrap();
        assert_eq!(
            (summary.runs_executed, summary.runs_reused),
            (3, 1),
            "{changed}"
        );
    }
    let summary = store::summary_path(&root, &f.lock.digest).unwrap();
    std::fs::write(&summary, b"invalid").unwrap();
    let r = f.prove(&p, 0).await;
    assert!(
        r.proofs
            .summary
            .unwrap()
            .refusals
            .contains(&"proofs_unreadable".into())
    );
}
#[tokio::test]
async fn partially_cached_group_keeps_stable_controls_when_budget_is_zero() {
    let mut f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    f.prove(&p, 10).await;
    let mut third = f.mutants.mutants[0].clone();
    third.patch = third.patch.replace("evil", "next");
    f.mutants.mutants.push(third);
    let r = f.prove(&p, 0).await;
    let s = r.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (0, 2));
    assert_eq!(r.proofs.runs.values().filter(|r| r.counted).count(), 2);
    assert_eq!(
        r.proofs
            .runs
            .values()
            .filter(|r| matches!(r.outcome, RunOutcome::BudgetExhausted))
            .count(),
        1
    );
    for run in r.proofs.runs.values().filter(|r| r.counted) {
        assert!(r.proofs.controls[&run.control_key].stable());
    }
    let r = f.prove(&p, 10).await;
    let s = r.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (3, 2));
}
#[tokio::test]
async fn persisted_output_preserves_non_text_bytes() {
    let f = Fixture::new("printf '\\377\\376'; test \"$(cat value)\" = good");
    let r = f.prove(&f.probe(true), 10).await;
    assert!(r.proofs.runs.values().all(|r| r.counted));
    for run in r.proofs.runs.values() {
        let bytes = std::fs::read(
            r.store
                .as_ref()
                .unwrap()
                .join(run.evidence.output.as_ref().unwrap()),
        )
        .unwrap();
        assert!(bytes.starts_with(&[0xff, 0xfe]), "{bytes:?}");
    }
}
#[tokio::test]
async fn zero_work_pre_control_is_a_mismatch() {
    let f = Fixture::new(
        "test -f value && printf 'running 0 tests\\ntest result: ok. 0 passed; 0 failed\\n'",
    );
    let r = f.prove(&f.probe(false), 10).await;
    let summary = r.proofs.summary.unwrap();
    assert_eq!(summary.runs_executed, 1);
    assert_eq!(summary.entries[0].state, ProofState::Unverified);
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| matches!(&r.outcome,RunOutcome::Unrun{code,..} if code=="control_mismatch"))
    );
    assert!(r.proofs.controls.values().all(
        |c| matches!(&c.pre,RunOutcome::Unrun{code,..} if code=="zero_work") && c.post.is_none()
    ));
}
