use super::super::*;
use super::*;

pub(crate) fn setup_evaluation(raw_os_error: i32) -> crate::command::workflow_gate::GateEvaluation {
    let trees = trees(&[("AC-X-001", "false", TrustedCwd::RepoRoot)]);
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut result: CheckResult = serde_json::from_value(serde_json::json!({
        "acceptance_id":"AC-X-001", "exit_code":null, "stdout":[], "stderr":[],
        "operational_error":"site rebuild failed; check is host-unproven: scratch/input: native I/O failure",
        "quota_walk_count":0
    })).unwrap();
    result.fault = serde_json::from_value(serde_json::json!({
        "cause":"site_setup_io", "raw_os_error":raw_os_error
    }))
    .ok();
    let text = result.operational_error.clone().unwrap();
    assert!(
        check_fault::settle(
            &probe,
            (Some(&trees.base), &contract, "AC-X-001"),
            check_fault::of_result(Some(&result)),
            "in the scratch site",
            text.clone()
        )
        .is_none()
    );
    let (_, cause) = probe.take_unproven_causes().remove("AC-X-001").unwrap();
    let finding = crate::command::workflow_gate::GateFinding::new(
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        text,
        "AC-X-001",
        None,
        archon_workflow::RemediationScope::Operational,
    )
    .with_defect(archon_workflow::defect::DeterministicDefect::new(
        cause.code(),
        "AC-X-001",
        "check",
    ));
    crate::command::workflow_gate::GateEvaluation::new("freeze", vec![finding])
}

pub(crate) fn replacement_evaluation() -> crate::command::workflow_gate::GateEvaluation {
    let trees = trees(&[("AC-X-001", "false", TrustedCwd::RepoRoot)]);
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let digest = contract_digest(&contract).unwrap();
    let refs = refs_for(&contract, &digest, &trees.ids());
    let result = serde_json::from_value(serde_json::json!({
        "acceptance_id":"AC-X-001", "exit_code":null, "stdout":[], "stderr":[],
        "operational_error":"scratch/input: native setup I/O failure",
        "fault":{"cause":"site_setup_io","raw_os_error":5}, "quota_walk_count":0
    }))
    .unwrap();
    let mut run = repairs::TreeRun::known(
        &hermetic::CheckEvidence::from_results(vec![result], None),
        "",
    );
    run.site_failed(
        &refs,
        hermetic::Unrun::error(
            std::io::Error::from_raw_os_error(28).into(),
            "scratch/required: free disk space for the site".into(),
        ),
    );
    run.site_failed(
        &refs,
        hermetic::Unrun::error(
            std::io::Error::from_raw_os_error(5).into(),
            "scratch/required: cold repair I/O failure".into(),
        ),
    );
    assert!(!run.results.contains_key("AC-X-001"));
    let text = run.unrun("AC-X-001").unwrap();
    assert!(
        check_fault::settle(
            &probe,
            (Some(&trees.base), &contract, "AC-X-001"),
            check_fault::Fault::Availability(run.fault_class),
            "in the scratch site",
            text,
        )
        .is_none()
    );
    let (text, cause) = probe.take_unproven_causes().remove("AC-X-001").unwrap();
    let finding = crate::command::workflow_gate::GateFinding::new(
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        text,
        "AC-X-001",
        None,
        archon_workflow::RemediationScope::Operational,
    )
    .with_defect(archon_workflow::defect::DeterministicDefect::new(
        cause.code(),
        "AC-X-001",
        "check",
    ));
    crate::command::workflow_gate::GateEvaluation::new("freeze", vec![finding])
}

#[tokio::test]
async fn acceptance_scratch_permanent_site_replacement_survives_later_transient_fault() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let envelope = replacement_evaluation().into_envelope().unwrap();
        assert_eq!(
            crate::command::workflow_host_fault::finding_class(&envelope.policy_findings[0]),
            crate::command::workflow_host_fault::Class::Permanent,
            "a cold repair must not downgrade the permanent site failure",
        );
        assert!(
            envelope.policy_findings[0]
                .text
                .contains("scratch/required: free disk space for the site")
        );
        assert!(
            !envelope.policy_findings[0]
                .text
                .contains("cold repair I/O failure")
        );
    })
    .await
    .expect("site replacement classification must finish");
}

#[tokio::test]
async fn acceptance_scratch_later_site_outage_replaces_only_transient_results() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let trees = trees(&[("AC-X-001", "rm src/state.txt; false", TrustedCwd::RepoRoot), ("AC-X-002", "false", TrustedCwd::RepoRoot), ("AC-X-003", "false", TrustedCwd::RepoRoot)]);
        std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
        std::fs::write(trees.set.project.path().join(".archon/config.toml"), format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(), trees.outside.path().join("scratch").display().to_string())).unwrap();
        let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
        let probe = std::sync::Arc::new(HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks).with_resume(&resume));
        let contract = trees.contract();
        let digest = contract_digest(&contract).unwrap();
        let mut refs = refs_for(&contract, &digest, &trees.ids());
        refs[2].command_digest = "invalid".into();
        let inject_probe = probe.clone();
        let predicates = BTreeMap::from([("AC-X-002".to_owned(), std::sync::Arc::new(move |_: &CheckResult| {
            inject_probe.injected_failures.store(2, std::sync::atomic::Ordering::SeqCst);
            true
        }) as resume::SavePredicate)]);
        let mut run = repairs::tree_results_with_save_predicates(&probe, &Baseline {
            commit: trees.base.clone(), repository: trees.repo.clone(),
        }, &contract, &digest, &refs, None, false, &predicates, &BTreeMap::new()).await;
        assert!(matches!(check_fault::of_result(Some(&run.results["AC-X-001"].result)), check_fault::Fault::Check { .. }), "an authored finding is retained");
        assert!(run.results.contains_key("AC-X-002"), "a completed verdict is retained");
        assert_eq!(run.fault_class, crate::command::workflow_host_fault::Class::Transient);
        assert!(run.results.contains_key("AC-X-003"), "a newer whole-site outage must retain the permanent refused reference: {:?}", run.results);
        let mut pending = refs.clone();
        for (id, raw_os_error) in [("AC-X-004", 5), ("AC-X-005", 28)] {
            let mut result = run.results["AC-X-003"].clone();
            result.result.acceptance_id = id.into();
            result.result.operational_error = Some("scratch/input: native setup I/O failure".into());
            result.result.fault = Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteSetupIo { raw_os_error });
            run.results.insert(id.into(), result);
            let mut reference = refs[2].clone();
            reference.acceptance_id = id.into();
            pending.push(reference);
        }
        run.site_failed(&pending, hermetic::Unrun::transient("the newer site attempt failed".into()));
        for id in ["AC-X-001", "AC-X-002", "AC-X-003", "AC-X-005"] {
            assert!(run.results.contains_key(id), "independent authored, completed, unknown and permanent evidence is retained: {id}");
        }
        assert!(!run.results.contains_key("AC-X-004"), "the stale transient setup result is replaced");
        assert!(run.unrun("AC-X-004").unwrap().contains("the newer site attempt failed"));
        let mut transient = run.results["AC-X-005"].clone();
        transient.result.acceptance_id = "AC-X-004".into();
        transient.result.fault = Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteSetupIo { raw_os_error: 5 });
        run.results.insert("AC-X-004".into(), transient);
        run.site_failed(&pending, hermetic::Unrun::unknown("scratch/required: restore the missing artifact".into()));
        assert!(!run.results.contains_key("AC-X-004"), "a newer permanent fault also replaces the stale transient observation");
        assert_eq!(run.fault_class, crate::command::workflow_host_fault::Class::Permanent);
        assert!(run.unrun("AC-X-004").unwrap().contains("scratch/required: restore the missing artifact"));
    }).await.expect("the configured scratch repair must finish");
}
