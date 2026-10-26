use super::*;

#[cfg(unix)]
#[tokio::test]
async fn real_repository_sibling_is_moved_in_scratch_and_live_bytes_stay_unchanged() {
    for cd in ["cd", "cd -P"] {
        assert_real_repository_sibling_is_moved_in_scratch(Some(cd), None).await;
    }
    for command in [
        "echo \"$(cat tests/focused.rs)\"",
        "fail() { :; }; fail \"pinned test $(tail -c 400 tests/focused.rs)\"",
        "echo \"`cat tests/focused.rs`\"",
        "test -f tests/focused.rs && echo 'tests/focused.rs'",
        "printf '%s\\n' tests/focused.rs | xargs grep -q focused",
        "echo tests/focused.rs | while read f; do grep -q focused \"$f\"; done",
        "V=\"$(printf %s tests/focused.rs)\"; [ -f \"$V\" ] || fail \"missing tests/focused.rs\"",
    ] {
        let command = format!("{command}; echo 'X PASS: pinned test (tests/focused.rs) ok'");
        assert_real_repository_sibling_is_moved_in_scratch(None, Some(&command)).await;
    }
}

#[cfg(unix)]
async fn assert_real_repository_sibling_is_moved_in_scratch(
    cd: Option<&str>,
    scenario: Option<&str>,
) {
    let repository_name = format!("repo-{}", uuid::Uuid::new_v4().simple());
    let (command, cwd) = match cd {
        Some(cd) => (
            format!(
                "{cd} ../{repository_name} && grep -q focused tests/focused.rs && echo 'X PASS: pinned test (tests/focused.rs) ok'"
            ),
            TrustedCwd::ProjectRoot,
        ),
        None => (scenario.unwrap().to_string(), TrustedCwd::RepoRoot),
    };
    let trees = trees(&[("AC-X-001", &command, cwd)]);
    let project = trees.set.project.path();
    let sibling_repository = project.parent().unwrap().join(&repository_name);
    let cloned = std::process::Command::new("git")
        .args(["clone", "-q"])
        .arg(&trees.repo)
        .arg(&sibling_repository)
        .output()
        .unwrap();
    assert!(
        cloned.status.success(),
        "{}",
        String::from_utf8_lossy(&cloned.stderr)
    );
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    let live = sibling_repository.join("tests/focused.rs");
    let original = std::fs::read(&live).unwrap();
    let scratch_parent = trees.outside.path().canonicalize().unwrap().join("scratch");
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            sibling_repository.display().to_string(), scratch_parent.display().to_string(),
        ),
    ).unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_reverify_with_limits(
        &project,
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
        repository: sibling_repository,
    };
    let digest = super::super::super::contract_digest(&contract).unwrap();
    let ids = BTreeSet::from(["AC-X-001".to_string()]);
    let refs = super::super::super::refs_for(&contract, &digest, &ids);
    let (evidence, _, missing, _) = probe.run_reverify_copy(&contract, &ids).await.unwrap();
    if cd.is_some() {
        assert!(missing.is_empty(), "{missing:?}");
    }
    let layout = evidence[0].copy_layout.as_ref().unwrap();
    if cd.is_some() {
        assert!(
            layout
                .site_repository
                .as_ref()
                .is_some_and(
                    |root| layout.check_siblings["AC-X-001"]
                        .iter()
                        .any(|sibling| sibling
                            .target
                            .as_ref()
                            .is_some_and(|target| target.starts_with(root)))
                ),
            "site repository root was not recorded: {layout:?}"
        );
    } else {
        let entry = contract.acceptance.first().unwrap();
        let discovery = super::super::super::mutation::input_discovery_for_check(
            entry,
            &probe.project,
            &baseline.repository,
            Some(layout),
        );
        let deciding_test = discovery
            .candidates
            .iter()
            .find(|candidate| candidate.path.ends_with("tests/focused.rs"))
            .unwrap_or_else(|| {
                panic!(
                    "the deciding test is not discovered for {command:?}: {:?}",
                    discovery.candidates
                )
            });
        assert_eq!(
            deciding_test.exclusion,
            Some(super::super::super::mutation::ExclusionReason::CheckMechanics),
            "a message with no movable class must remain mechanics for {command:?}: {deciding_test:?}"
        );
        assert!(
            !discovery
                .eligible
                .iter()
                .any(|candidate| candidate.ends_with("tests/focused.rs"))
        );
        return;
    }
    if cd.is_none() {
        return;
    }
    let finding = super::super::super::prove::prove(
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
        finding.is_empty(),
        "the check should fail when its copied deciding test is moved: {finding:?}"
    );
    assert!(probe.take_unproven().is_empty());
    assert_eq!(
        std::fs::read(&live).unwrap(),
        original,
        "Scratch prove changed the live repository test"
    );
}
