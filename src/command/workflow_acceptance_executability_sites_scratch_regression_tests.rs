#[tokio::test]
async fn scratch_mutation_covers_a_child_named_before_its_parent() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f tests/focused.rs && test -d tests",
        TrustedCwd::RepoRoot,
    )]);
    let project = trees.set.project.path();
    let live_input = trees.repo.join("tests/focused.rs");
    let original_input = std::fs::read(&live_input).unwrap();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::write(
        project.join(".archon/config.toml"),
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
        project,
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    assert!(matches!(probe.site, Site::Scratch(_)));
    let contract = trees.contract();
    let baseline = Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let ids = BTreeSet::from(["AC-X-001".to_string()]);
    let (observed, _, missing, _) = probe.run_reverify_copy(&contract, &ids).await.unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let layout = observed[0].copy_layout.as_ref().unwrap();
    let entry = &contract.acceptance[0];
    let discovery = super::super::mutation::input_discovery_for_check(
        entry,
        project,
        &trees.repo,
        Some(layout),
    );
    assert_eq!(discovery.eligible, ["tests/focused.rs", "tests"]);
    let markers = super::super::mutation::Markers::new();
    let plan = super::super::prove::mutation_plan(&discovery.eligible, &discovery, entry, &markers);
    assert_eq!(plan.inputs.len(), 2);
    assert_eq!(plan.inputs[0].named_path, "tests");
    assert_eq!(plan.inputs[1].named_path, "tests/focused.rs");
    let names = plan
        .inputs
        .iter()
        .map(|input| input.named_path.clone())
        .collect::<Vec<_>>();
    let input_map = BTreeMap::from([("AC-X-001".to_string(), names.clone())]);
    let move_paths = BTreeMap::from([(
        "AC-X-001".to_string(),
        plan.inputs
            .iter()
            .map(|input| {
                discovery
                    .candidates
                    .iter()
                    .find(|candidate| candidate.path == input.named_path)
                    .and_then(|candidate| candidate.move_path.clone())
                    .unwrap_or_else(|| input.named_path.clone())
            })
            .collect(),
    )]);
    let move_roots = BTreeMap::from([(
        "AC-X-001".to_string(),
        plan.inputs
            .iter()
            .map(|input| {
                discovery
                    .candidates
                    .iter()
                    .find(|candidate| candidate.path == input.named_path)
                    .and_then(|candidate| candidate.move_root.clone())
            })
            .collect(),
    )]);
    let live_roots = [project.canonicalize().unwrap(), trees.repo.canonicalize().unwrap()];
    let mutated = super::super::mutation::mutated(
        &contract,
        &input_map,
        &move_paths,
        &move_roots,
        &[live_roots[0].as_path(), live_roots[1].as_path()],
        &markers,
    );
    let mutated_digest = super::super::contract_digest(&mutated).unwrap();
    let mutated_refs = super::super::refs_for(&mutated, &mutated_digest, &ids);
    let run = super::super::sites::run_at(
        &probe,
        &baseline,
        &mutated,
        &mutated_digest,
        &mutated_refs,
        false,
        &BTreeMap::new(),
        &BTreeMap::from([("AC-X-001".to_string(), plan)]),
    )
    .await
    .unwrap();
    let evidence = run.results.into_iter().next().unwrap();
    let result = &evidence.result;
    assert_eq!(result.exit_code, Some(1));
    assert!(result.operational_error.is_none(), "{:?}", result.operational_error);
    let states = super::super::prove::input_states(result, &markers, &names);
    assert_eq!(
        states.get("tests").map(String::as_str),
        Some("restored"),
        "{states:?}; stdout={}; stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(super::super::prove::all_moved_inputs_restored(result, &markers, &names));
    let move_states = markers.move_states(result, &names);
    let child_diagnostics = super::super::prove::describe_candidates_with_run_state(
        &discovery.candidates,
        &BTreeSet::new(),
        &[],
        &markers.moved(result, &names).into_iter().collect(),
        &BTreeSet::new(),
        &move_states,
        &BTreeMap::from([(String::from("tests/focused.rs"), String::from("tests"))]),
        true,
    );
    assert!(
        child_diagnostics.contains("covered by moved ancestor 'tests': tests/focused.rs"),
        "{child_diagnostics}"
    );
    assert!(super::super::prove::mutation_host_or_author_finding(
        &evidence,
        &markers,
        &names,
        None,
        "the check passed before mutation",
    )
    .is_none());
    let original_digest = super::super::contract_digest(&contract).unwrap();
    let original_refs = super::super::refs_for(&contract, &original_digest, &ids);
    let findings = super::super::prove::prove(
        &probe,
        &baseline,
        &contract,
        &original_digest,
        &original_refs,
        &["AC-X-001".to_string()],
        &BTreeMap::from([("AC-X-001".to_string(), layout.clone())]),
    )
    .await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(probe.take_unproven().is_empty());
    assert_eq!(std::fs::read(&live_input).unwrap(), original_input);
}
