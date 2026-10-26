//! Regressions for the configured Scratch execution site.

use super::*;
use crate::command::workflow_task_set::executability::probe_tests::trees;
use archon_workflow::task_set_contract::TrustedCwd;

#[path = "workflow_acceptance_executability_sites_scratch_message_tests.rs"]
mod message_tests;

#[tokio::test]
async fn unrestored_mutation_results_are_not_saved_or_memoized_across_same_key_runs() {
    for (command, expected_state) in [
        (
            "printf recreated > data/state.txt; false",
            "original-recreated",
        ),
        ("rm -rf data; false", "aside-missing"),
    ] {
        let trees = trees(&[("AC-M-001", command, TrustedCwd::ProjectRoot)]);
        let copies = tempfile::tempdir().unwrap();
        let resume =
            crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
        let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_resume(&resume)
            .with_copy_parent(copies.path().to_path_buf());
        let baseline = Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        };
        let original = trees.contract();
        let markers = super::super::mutation::Markers::new();
        let names = vec!["data/state.txt".to_string()];
        let inputs = BTreeMap::from([("AC-M-001".to_string(), names.clone())]);
        let contract = super::super::mutation::mutated(
            &original,
            &inputs,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &markers,
        );
        let digest = super::super::contract_digest(&contract).unwrap();
        let refs = super::super::refs_for(&contract, &digest, &trees.ids());
        let save_markers = markers.clone();
        let save_names = names.clone();
        let predicates = BTreeMap::from([(
            "AC-M-001".to_string(),
            Arc::new(
                move |result: &archon_workflow::acceptance_scratch::CheckResult| {
                    super::super::prove::all_moved_inputs_restored(
                        result,
                        &save_markers,
                        &save_names,
                    )
                },
            ) as super::super::resume::SavePredicate,
        )]);
        let data = data_state(&probe, &baseline).unwrap();
        let key = reuse::memo_key(&probe, &baseline, &data, &contract, "AC-M-001").unwrap();
        memo().lock().unwrap().remove(&key);

        let mutation_plan = archon_workflow::acceptance_scratch::MutationPlan {
            nonce: markers.nonce().to_string(),
            inputs: vec![archon_workflow::acceptance_scratch::MutationInputPlan {
                named_path: names[0].clone(),
                site_copy_alias: false,
                move_path: PathBuf::from("project").join(&names[0]),
                aside_path: PathBuf::from(format!(
                    "project/{}.archon-mutated-{}",
                    names[0],
                    markers.nonce().get(..12).unwrap()
                )),
            }],
        };
        let mutation_plans = BTreeMap::from([("AC-M-001".to_string(), mutation_plan)]);
        let (first, second) = tokio::join!(
            run_at(
                &probe,
                &baseline,
                &contract,
                &digest,
                &refs,
                false,
                &predicates,
                &mutation_plans
            ),
            run_at(
                &probe,
                &baseline,
                &contract,
                &digest,
                &refs,
                false,
                &predicates,
                &mutation_plans
            ),
        );
        let store = probe.store().unwrap();
        let path = store.path(&key).unwrap();
        assert!(first.is_ok() && second.is_ok());
        for result in first.into_iter().chain(second) {
            let result = result.results.into_iter().next().unwrap().result;
            let states = super::super::prove::input_states(&result, &markers, &names);
            assert_eq!(
                states.get(&names[0]).map(String::as_str),
                Some(expected_state)
            );
        }
        assert!(!path.with_extension("provisional").exists());
        assert!(!path.exists());
        assert!(!memo().lock().unwrap().contains_key(&key));
    }
}

#[tokio::test]
async fn scratch_layout_unions_all_checks_and_keeps_each_check_siblings() {
    let trees = trees(&[
        (
            "AC-S-001",
            "grep -q ready ../sibling-a/fixture.txt",
            TrustedCwd::RepoRoot,
        ),
        (
            "AC-S-002",
            "grep -q ready ../sibling-b/fixture.txt && false",
            TrustedCwd::RepoRoot,
        ),
        ("AC-S-003", "test -f src/state.txt", TrustedCwd::RepoRoot),
    ]);
    for name in ["sibling-a", "sibling-b"] {
        let root = trees.outside.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("fixture.txt"), "ready\n").unwrap();
    }
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch_parent.display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_reverify_with_limits(
        trees.set.project.path(),
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    assert!(matches!(probe.site, Site::Scratch(_)));

    let contract = trees.contract();
    let (evidence, _, missing, _) = probe
        .run_reverify_copy(&contract, &trees.ids())
        .await
        .expect("Scratch reverify observation");
    assert!(missing.is_empty(), "{missing:?}");
    let second = evidence
        .iter()
        .find(|item| item.acceptance_id == "AC-S-002")
        .expect("second sibling check ran");
    let layout = second.copy_layout.as_ref().expect("Scratch layout");
    assert!(
        layout
            .siblings
            .iter()
            .any(|sibling| sibling.path == Path::new("../sibling-a"))
    );
    assert!(
        layout
            .siblings
            .iter()
            .any(|sibling| sibling.path == Path::new("../sibling-b"))
    );
    assert_eq!(
        layout.check_siblings["AC-S-001"].len(),
        1,
        "layout siblings: {:?}; check map: {:?}",
        layout.siblings,
        layout.check_siblings
    );
    assert_eq!(
        layout.check_siblings["AC-S-001"][0].path,
        Path::new("../sibling-a")
    );
    assert_eq!(layout.check_siblings["AC-S-002"].len(), 1);
    assert_eq!(
        layout.check_siblings["AC-S-002"][0].path,
        Path::new("../sibling-b")
    );
    assert!(layout.check_siblings["AC-S-003"].is_empty());
    assert_eq!(second.exit_code, Some(1));
    assert!(second.operational_error.is_none());
    let at = super::super::silent::baseline_context(&probe, &contract, Some(layout));
    assert_eq!(
        super::super::silent::silent_failure(&contract, &second.result, &at),
        None,
        "the second check's behavioral failure is not discarded as setup"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn project_root_repository_alias_input_is_moved_and_restored_at_scratch_site() {
    let repository_alias = format!("repo-{}", uuid::Uuid::new_v4().simple());
    let command = format!(
        "cd ../{repository_alias} && echo 'tests//focused.rs' && test -f tests/focused.rs && grep -q focused tests/focused.rs"
    );
    let trees = trees(&[("AC-X-001", &command, TrustedCwd::ProjectRoot)]);
    let project_parent = trees.set.project.path().parent().unwrap();
    std::os::unix::fs::symlink(&trees.repo, project_parent.join(&repository_alias)).unwrap();
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch_parent.display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_reverify_with_limits(
        trees.set.project.path(),
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    assert!(matches!(probe.site, Site::Scratch(_)));

    let contract = trees.contract();
    let (baseline_evidence, _, missing, _) = probe
        .run_reverify_copy(&contract, &trees.ids())
        .await
        .expect("the production Scratch observer builds the baseline site");
    assert!(missing.is_empty(), "{missing:?}");
    let baseline_check = baseline_evidence
        .iter()
        .find(|item| item.acceptance_id == "AC-X-001")
        .expect("Scratch check evidence");
    let layout = baseline_check
        .copy_layout
        .as_ref()
        .expect("the production Scratch observer records its site layout");
    let entry = contract.acceptance.first().unwrap();
    let discovery = super::super::mutation::input_discovery_for_check(
        entry,
        &probe.project,
        &trees.repo,
        Some(layout),
    );
    assert!(
        discovery
            .eligible
            .iter()
            .any(|name| name.ends_with("repo/tests/focused.rs")),
        "repository test file was not discovered: {:?}; candidates: {:?}",
        discovery.eligible,
        discovery.candidates
    );
    assert!(
        discovery.candidates.iter().any(|candidate| {
            candidate.exclusion == Some(super::super::mutation::ExclusionReason::CheckMechanics)
        }),
        "cd operand must remain a check mechanic: {:?}",
        discovery.candidates
    );
    assert!(
        !discovery
            .eligible
            .iter()
            .any(|name| name == &format!("../{repository_alias}"))
    );
    assert!(
        layout.check_siblings["AC-X-001"]
            .first()
            .and_then(|sibling| sibling.target.as_ref())
            .is_some(),
        "Scratch observer did not capture sibling target: {:?}",
        layout
    );

    let baseline = Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let findings = super::super::prove::prove(
        &probe,
        &baseline,
        &contract,
        &digest,
        &refs,
        &["AC-X-001".to_string()],
        &BTreeMap::from([("AC-X-001".to_string(), layout.clone())]),
    )
    .await;
    assert!(
        findings.is_empty(),
        "mutated file must fail the check, restore cleanly, and never try to move the cd target: {findings:?}"
    );
    assert!(probe.take_unproven().is_empty());
}

#[tokio::test]
async fn normal_run_saves_and_reuses_evidence_with_an_existing_lock_path() {
    let trees = trees(&[(
        "AC-M-001",
        "grep -q ready data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_resume(&resume)
        .with_copy_parent(copies.path().to_path_buf());
    let baseline = Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let contract = trees.contract();
    let ids = trees.ids();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &ids);
    let key = reuse::memo_key(
        &probe,
        &baseline,
        &data_state(&probe, &baseline).unwrap(),
        &contract,
        "AC-M-001",
    )
    .unwrap();
    let store = probe.store().unwrap();
    let result_path = std::path::PathBuf::from(store.evidence_ref(&key).unwrap());
    std::fs::create_dir_all(result_path.parent().unwrap()).unwrap();
    // A normal run should still produce and reuse evidence when this path exists.
    std::fs::create_dir(result_path.with_extension("lock")).unwrap();
    let run = run_at(
        &probe,
        &baseline,
        &contract,
        &digest,
        &refs,
        false,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .await
    .expect("normal run should not fail on obsolete lock state");
    assert_eq!(run.results.len(), 1);
    assert!(store.load(&key, false).is_some());
}

include!("workflow_acceptance_executability_sites_scratch_regression_tests.rs");
