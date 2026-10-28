use super::*;

#[cfg(unix)]
#[tokio::test]
async fn host_command_acceptance_scratch_permanent_replacement_returns_once_with_remedy() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let fixture = fixture(0, false);
        let evaluation = crate::command::workflow_task_set::executability::probe_tests::scratch_rebuild_tests::replacement_evaluation();
        let remedy = evaluation.findings[0].text.clone();
        *fixture.process.evaluation.lock().unwrap() = Some(evaluation);
        run(&fixture).await.unwrap();
        assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1,
            "a later transient cold repair must not retry the permanent site failure");
        let recorded = records(&fixture);
        assert_eq!(recorded.len(), 1);
        let error = &recorded[0].result.data["gateEnvelope"]["operational_error"];
        assert_eq!(error["kind"], "permanent_host_fault");
        assert_eq!(error["text"], remedy);
        assert!(remedy.contains("scratch/required: free disk space for the site"));
        assert!(!remedy.contains("cold repair I/O failure"));
        assert!(recorded[0].result.data["publicationReceipt"].is_null());
    }).await.expect("permanent site replacement dispatch must finish");
}

#[cfg(unix)]
#[tokio::test]
async fn host_command_scratch_rebuild_native_outage_heals_before_recording() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let healed = fixture(0, false);
        *healed.process.evaluation.lock().unwrap() = Some(
            crate::command::workflow_task_set::executability::probe_tests::scratch_rebuild_tests::setup_evaluation(5),
        );
        run(&healed).await.unwrap();
        assert_eq!(healed.process.calls.load(Ordering::SeqCst), 2,
            "a native rebuild outage must retry inside the persisted call");
        let recorded = records(&healed);
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].result.data["publicationReceipt"].is_object());
        assert!(recorded[0].result.data["gateEnvelope"]["operational_error"].is_null());
        let permanent = fixture(0, false);
        *permanent.process.evaluation.lock().unwrap() = Some(
            crate::command::workflow_task_set::executability::probe_tests::scratch_rebuild_tests::setup_evaluation(28),
        );
        run(&permanent).await.unwrap();
        assert_eq!(permanent.process.calls.load(Ordering::SeqCst), 1);
        let recorded = records(&permanent);
        let error = &recorded[0].result.data["gateEnvelope"]["operational_error"];
        assert_eq!(error["kind"], "permanent_host_fault");
        assert_eq!(error["text"], "site rebuild failed; check is host-unproven: scratch/input: native I/O failure");
    }).await.expect("scratch rebuild dispatch must finish");
}

#[cfg(unix)]
#[tokio::test]
async fn host_command_mixed_real_gate_returns_authored_findings_immediately() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use archon_workflow::task_set_contract::TrustedCwd;
    let trees = trees(&[
        (
            "AC-X-001",
            "sleep 30; test -f feature.txt",
            TrustedCwd::RepoRoot,
        ),
        (
            "AC-X-002",
            "rm -f target && test -f feature.txt",
            TrustedCwd::RepoRoot,
        ),
    ]);
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(trees.set.project.path().join(".archon/config.toml"), format!(
        "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=3\noutput_bytes=8192\nscratch_bytes=16777216\n",
        trees.repo.display().to_string(), trees.outside.path().join("scratch").display().to_string(),
    )).unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probed = crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.project.path().join("input.md"),
        &trees.contract(),
        &resume,
    )
    .await
    .unwrap();
    assert_eq!(probed.findings.len(), 2);
    assert!(
        probed
            .findings
            .iter()
            .any(|finding| finding.remediation_scope
                == archon_workflow::RemediationScope::CandidateArtifact),
        "{:?}",
        probed.findings
    );
    assert!(
        probed
            .findings
            .iter()
            .any(|finding| finding.remediation_scope
                == archon_workflow::RemediationScope::Operational),
        "{:?}",
        probed.findings
    );
    let fixture = fixture(0, false);
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow_gate::GateEvaluation::new("freeze", probed.findings),
    );
    let outcome = run_script(&fixture, r#"async function workflow(w) { return await w.hostCommand("freeze-acceptance", { stdin: "{}" }); }"#).await;
    assert_eq!(
        fixture.process.calls.load(Ordering::SeqCst),
        1,
        "{outcome:?}"
    );
    outcome.unwrap();
    assert_eq!(
        records(&fixture)[0].result.data["gateEnvelope"]["policy_findings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[cfg(unix)]
#[tokio::test]
async fn host_command_site_lacks_program_is_not_an_install_remedy() {
    use crate::command::workflow_task_set::executability::{HOST_PATH, probe_tests::trees};
    use archon_workflow::task_set_contract::TrustedCwd;
    let trees = trees(&[(
        "AC-X-001",
        "capability-x && test -f feature.txt",
        TrustedCwd::RepoRoot,
    )]);
    let bin = trees.outside.path().join("host-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = bin.join("capability-x");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(trees.set.project.path().join(".archon/config.toml"), format!(
        "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
        trees.repo.display().to_string(), trees.outside.path().join("scratch").display().to_string(),
    )).unwrap();
    HOST_PATH.with(|path| *path.borrow_mut() = Some(format!("{}:/usr/bin:/bin", bin.display())));
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probed = crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.project.path().join("input.md"),
        &trees.contract(),
        &resume,
    )
    .await
    .unwrap();
    HOST_PATH.with(|path| *path.borrow_mut() = None);
    assert_eq!(probed.findings.len(), 1);
    assert_eq!(
        probed.findings[0].remediation_scope,
        archon_workflow::RemediationScope::Operational
    );
    assert!(
        probed.findings[0]
            .text
            .contains("its site lacks what the host has"),
        "{:?}",
        probed.findings
    );
    let fixture = fixture(0, false);
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow_gate::GateEvaluation::new("freeze", probed.findings),
    );
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[cfg(unix)]
#[tokio::test]
async fn host_command_configured_scratch_busy_lease_recovers_after_settlement_window() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use archon_workflow::task_set_contract::TrustedCwd;
    let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(trees.set.project.path().join(".archon/config.toml"), format!(
        "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
        trees.repo.display().to_string(), trees.outside.path().join("scratch").display().to_string())).unwrap();
    let identity = archon_shell::paths::canonicalize(&trees.repo).unwrap();
    let lease = crate::command::acceptance_scratch_guardian::acquire_lease(
        &std::env::temp_dir().join("archon-native-observer-locks"),
        &identity.to_string_lossy(),
    )
    .unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probed = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
            trees.set.project.path(),
            &trees.set.tasks,
            &trees.set.project.path().join("input.md"),
            &trees.contract(),
            &resume,
        ),
    )
    .await
    .expect("busy lease must yield a classified observation")
    .unwrap();
    drop(lease);
    assert!(!probed.findings.is_empty());
    let fixture = fixture(0, false);
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow_gate::GateEvaluation::new("freeze", probed.findings),
    );
    run(&fixture).await.unwrap();
    assert_eq!(
        fixture.process.calls.load(Ordering::SeqCst),
        2,
        "temporary lease contention must retry"
    );
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}
