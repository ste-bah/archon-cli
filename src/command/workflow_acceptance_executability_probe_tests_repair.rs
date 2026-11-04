use super::*;

/// (3) A probe run the host could not complete is repaired before it is
/// anyone's finding: a plain re-run on a fresh copy recovers it, and every
/// repair is recorded.
#[tokio::test]
async fn a_probe_run_the_host_could_not_complete_is_repaired_and_recorded() {
    let trees = trees(&[("AC-E-001", "test -f feature.txt", REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let round = |failures: usize| {
        HostProbe::at(
            trees.set.project.path().to_path_buf(),
            trees.repo.clone(),
            None,
        )
        .with_baseline(super::Baseline {
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        })
        .with_copy_parent(copies.path().to_path_buf())
        .with_injected_failures(failures)
    };
    let probe = round(1);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert!(findings.is_empty(), "recovered by the re-run: {findings:?}");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("host-environment repair of the probe at")
                && d.contains("re-ran it on a freshly built copy")
                && d.contains("injected host failure")),
        "{diagnostics:?}"
    );
    // Fix 6: still unrunnable after every repair: the host's, never the
    // author's -- unproven, with the repair log. A copy has no build cache,
    // so one repair is all there is.
    let probe = round(5);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "never an author finding: {findings:?}");
    let unproven = probe.take_unproven();
    let why = &unproven["AC-E-001"];
    assert!(
        why.contains("could not be run on the pre-implementation tree")
            && why
                .contains("after repairing its own environment: re-ran it on a freshly built copy"),
        "{why}"
    );
    assert_eq!(
        probe
            .injected_failures
            .load(std::sync::atomic::Ordering::SeqCst),
        3,
        "one run and one repaired re-run"
    );
    trees.assert_live_untouched(copies.path());
}

/// (3) With a scratch site that has a build cache, the second repair runs
/// without it.
#[tokio::test]
async fn a_scratch_probe_that_still_cannot_run_is_rerun_without_its_build_cache() {
    let trees = trees(&[("AC-E-001", "test -f built.txt", REPO)]);
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
        ),
    )
    .unwrap();
    let cache = trees.outside.path().join("build-cache");
    let mut binding = crate::command::acceptance_scratch_policy::capture(
        trees.set.project.path(),
        &trees.set.tasks,
    )
    .unwrap()
    .expect("the configured scratch site");
    binding.policy.build_cache = Some(cache.clone());
    // The site run and its plain re-run fail; the baseline is the site's
    // own HEAD, so its verdict is the site's.
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        Some(binding),
    )
    .with_baseline(super::Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        commit: trees.head.clone(),
        repository: trees.repo.clone(),
    })
    .with_injected_failures(2);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert!(findings.is_empty(), "{findings:?} {diagnostics:?}");
    let repairs: Vec<&String> = (diagnostics.iter())
        .filter(|d| d.contains("host-environment repair of the probe at"))
        .collect();
    assert_eq!(repairs.len(), 2, "{diagnostics:?}");
    assert!(
        repairs[1].contains("without the run's compiled-artifact cache"),
        "{repairs:?}"
    );
    assert!(!cache.exists(), "the cold re-run never used the cache");
    assert!(
        scratch_left(&scratch).is_empty(),
        "scratch removed: {:?}",
        scratch_left(&scratch)
    );
}

async fn timed_out_repair(probe: &HostProbe, trees: &Trees) -> BTreeMap<String, String> {
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "repair".into(),
        exit_code: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        verdict_streams: Default::default(),
        fault: None,
        quota_walk_count: 0,
        operational_error: Some(archon_workflow::acceptance_scratch::CHECK_TIMED_OUT.into()),
        classification: None,
        environment_note: None,
    };
    let known = [super::super::hermetic::CheckEvidence {
        result,
        copy_layout: None,
    }];
    let failed = FailedTree {
        commit: Some(trees.base.clone()),
        originals: [("repair".into(), Original::Failed)].into(),
    };
    let mut findings = super::super::baseline::newly_passing_findings(
        probe,
        &failed,
        &contract,
        &digest,
        &refs,
        Some(&known),
    )
    .await;
    // The normal freeze also visits the baseline; do not count one observation twice.
    let remaining: Vec<_> = refs
        .into_iter()
        .filter(|reference| {
            !probe.is_unproven(&reference.acceptance_id)
                && !findings.contains_key(&reference.acceptance_id)
        })
        .collect();
    findings.extend(
        super::super::baseline::cannot_fail_findings(
            probe,
            &Baseline {
                origin:
                    crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
                commit: trees.base.clone(),
                repository: trees.repo.clone(),
            },
            &contract,
            &digest,
            &remaining,
            Some(&known),
        )
        .await,
    );
    findings
}
#[tokio::test]
#[allow(non_snake_case)]
async fn P2_21_first_repair_timeout_records_stall_strike() {
    let trees = trees(&[("repair", "sleep 3; test -f feature.txt", REPO)]);
    let mut probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_baseline(Baseline {
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        })
        .with_failed_tree(FailedTree {
            commit: Some(trees.head.clone()),
            originals: [("repair".into(), Original::Failed)].into(),
        });
    probe.check_cap_secs = 1;
    assert!(
        probe
            .script_defects(&trees.contract(), &trees.ids())
            .await
            .is_empty()
    );
    let unproven = probe.take_unproven();
    assert!(unproven["repair"].contains("timed out"));
    assert!(!unproven["repair"].contains("twice"));
}
#[tokio::test]
#[allow(non_snake_case)]
async fn P2_22_second_repair_timeout_after_resume_reaches_author() {
    let trees = trees(&[("repair", "test -f feature.txt", REPO)]);
    let probe = || HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    assert!(timed_out_repair(&probe(), &trees).await.is_empty());
    let resumed = probe();
    assert!(timed_out_repair(&resumed, &trees).await["repair"].contains("made no progress"));
    assert!(resumed.take_unproven().is_empty());
}

#[test]
#[allow(non_snake_case)]
fn P2_26_direct_error_note_does_not_claim_proof_without_base() {
    assert_eq!(
        super::super::probe::direct_fallback_note(),
        "it is proven on the hermetic trees when a pre-implementation tree exists; otherwise unproven"
    );
}

#[cfg(unix)]
#[tokio::test]
#[allow(non_snake_case)]
async fn S2_package_findings_never_escape_the_requested_ids() {
    // Input: a passing check is asked; a directory/package mismatch is not.
    // Expected: all three probe routes return findings only for the asked id.
    let trees = trees(&[
        ("asked", "test -f src/state.txt", REPO),
        ("unasked", "cargo test -p repo", REPO),
    ]);
    std::fs::write(
        trees.repo.join("Cargo.toml"),
        "[package]\nname='real-package'\nversion='0.1.0'\n",
    )
    .unwrap();
    git(&trees.repo, &["add", "."]);
    git(&trees.repo, &["commit", "-qm", "manifest"]);
    let base = git(&trees.repo, &["rev-parse", "HEAD"]);
    let contract = trees.contract();
    assert!(
        super::super::super::passability::package_findings(
            &contract,
            &trees.repo,
            trees.set.project.path(),
            &base,
            false,
        )
        .contains_key("unasked")
    );
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_baseline(Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        commit: base,
        repository: trees.repo.clone(),
    });
    let ids = BTreeSet::from(["asked".into()]);
    for findings in [
        probe.script_defects(&contract, &ids).await,
        probe.hold_originals(&contract, &ids).await,
        probe.prove_can_fail(&contract, &ids).await.findings,
    ] {
        assert!(findings.keys().all(|id| ids.contains(id)), "{findings:?}");
    }
}
