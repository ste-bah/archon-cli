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
            && why.contains(
                "after the host repaired its environment: re-ran it on a freshly built copy"
            ),
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
