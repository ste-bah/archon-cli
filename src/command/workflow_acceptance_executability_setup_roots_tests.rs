//! Issue 495: the setup-path rule decides "outside the tree" from the names
//! the real scratch site holds at its top, never from a second list. A real
//! check lists the top of the real site it ran in; the rule must call a
//! missing path below a listed name inside the tree, and one below any
//! other name outside it.

use super::*;
use crate::command::workflow_task_set::executability::probe_tests::trees;
use archon_workflow::task_set_contract::TrustedCwd;
use std::collections::BTreeSet;

/// The fixture: one check that lists the top of the site it ran in, with
/// a scratch policy configured.
fn listing_fixture() -> crate::command::workflow_task_set::executability::probe_tests::Trees {
    // `..` alone names no sibling, so the site copies nothing for it. The
    // check window (a no-progress limit) outlasts nextest's per-test
    // ceiling, so a child scheduled late under load cannot be stopped.
    let trees = trees(&[("AC-ROOTS-001", "cd .. && ls -1A", TrustedCwd::RepoRoot)]);
    let project = trees.set.project.path();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=300\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch_parent.display().to_string(),
        ),
    )
    .unwrap();
    trees
}

/// The rule must call a missing path below a name outside the tree exactly
/// when the real site's top (`listing`) does not hold that name.
fn assert_agreement(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    listing: &CheckEvidence,
) -> BTreeSet<String> {
    assert_eq!(listing.exit_code, Some(0), "{listing:?}");
    let held: BTreeSet<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    // Controls: the selected root is held, an invented name never is.
    assert!(held.contains("repo"), "{held:?}");
    let mut names: BTreeSet<String> = held.clone();
    names.extend(
        archon_workflow::acceptance_scratch::SITE_INTERNAL_DIRECTORIES
            .iter()
            .map(|name| (*name).to_string()),
    );
    names.insert("archon-not-a-site-root".into());
    let at = baseline_context(probe, contract, listing.copy_layout.as_ref());
    for name in &names {
        let path = format!("../{name}/archon-absent-dir");
        // A result of its own, so the streams set below are the ones read.
        let mut failed = listing.result.for_verdict();
        failed.classification = None;
        failed.exit_code = Some(1);
        failed.stdout = Vec::new();
        failed.stderr = format!("sh: cd: {path}: No such file or directory\n").into_bytes();
        let refusal =
            super::super::verdict::setup_path::failure(&format!("cd {path}"), &failed, &at);
        assert_eq!(
            refusal.is_some(),
            !held.contains(name),
            "`../{name}`: the real site top holds {held:?}, but the rule said {refusal:?}"
        );
    }
    held
}

/// The reverify run builds cold: the site keeps its build target at its top.
#[tokio::test]
async fn setup_path_context_agrees_with_actual_site_roots() {
    let trees = listing_fixture();
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
    let (observed, _, missing, _) = probe
        .run_reverify_copy(&contract, &trees.ids())
        .await
        .unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let listing = &observed[0];
    assert_eq!(
        listing
            .copy_layout
            .as_ref()
            .and_then(|layout| layout.site_build_cache),
        Some(false),
        "a cold site"
    );
    let held = assert_agreement(&probe, &contract, listing);
    assert!(
        held.contains("target"),
        "a cold site keeps its target: {held:?}"
    );
}

/// A warm run keeps the build target in the run's build cache, so the site
/// top holds no `target`: a missing path below `../target` lies outside it.
#[tokio::test]
async fn setup_path_context_agrees_with_cached_site_roots() {
    let trees = listing_fixture();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_resume(&resume)
        .with_host_environment(crate::test_environment::probe_host())
        .with_copy_parent(copies.path().to_path_buf())
        .without_process_memo();
    let Site::Scratch(binding) = &probe.site else {
        panic!("a configured scratch site");
    };
    assert!(
        binding.policy.build_cache.is_some(),
        "the freeze site is warm"
    );
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let run = super::super::sites::run_at(
        &probe,
        &Baseline {
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &refs,
        false,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .await
    .unwrap_or_else(|error| panic!("the warm site ran: {}", error.0));
    let listing = &run.results[0];
    assert_eq!(
        listing
            .copy_layout
            .as_ref()
            .and_then(|layout| layout.site_build_cache),
        Some(true),
        "a warm site"
    );
    let held = assert_agreement(&probe, &contract, listing);
    assert!(
        !held.contains("target"),
        "a warm site has no target at its top: {held:?}"
    );
}
