//! Issue 420 through a real freeze probe on real trees: an existing sibling
//! named by a check is materialized into the probe copy before the check runs.

use super::probe_tests::trees;
use super::verdict_probe_tests::{REPO, round_at};
use super::{ExecutabilityProbe, HostProbe};

#[tokio::test]
async fn a_missing_repository_sibling_path_is_an_author_finding() {
    let command =
        "ROOT=1 cargo metadata --no-deps --format-version 1 --manifest-path ../absent/Cargo.toml";
    let trees = trees(&[("AC-420-001", command, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let finding = findings.get("AC-420-001").expect("missing path finding");
    assert!(finding.contains("../absent/Cargo.toml"), "{finding}");
    assert!(
        unproven.is_empty(),
        "author findings are not probe verdicts"
    );
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_same_commit_known_copy_result_keeps_its_layout_for_judgment() {
    let command = "cargo metadata --no-deps --format-version 1 --manifest-path ../archon-446-outside/Cargo.toml";
    let trees = trees(&[("AC-446-001", command, REPO)]);
    let sibling = trees.outside.path().join("archon-446-outside");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join("Cargo.toml"),
        "[package]\nname='outside'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::create_dir_all(sibling.join("src")).unwrap();
    std::fs::write(sibling.join("src/lib.rs"), "pub fn value() {}\n").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_baseline(super::Baseline {
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        })
        .with_copy_parent(copies.path().to_path_buf());

    let contract = trees.contract();
    let digest = super::contract_digest(&contract).unwrap();
    let refs = super::refs_for(&contract, &digest, &trees.ids());
    let first = probe.run_with_layout(&contract, &digest, &refs).await;
    let memo_hit = probe.run_with_layout(&contract, &digest, &refs).await;
    let revisions = &memo_hit.results[0]
        .copy_layout
        .as_ref()
        .expect("memo retains copy layout")
        .siblings;
    assert_eq!(revisions.len(), 1);
    assert_eq!(
        revisions[0].path,
        std::path::PathBuf::from("../archon-446-outside")
    );
    assert_eq!(
        first.results[0]
            .copy_layout
            .as_ref()
            .unwrap()
            .siblings
            .len(),
        1
    );
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_hermetic_memo_hit_keeps_its_layout_for_judgment() {
    use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

    let command = "test -f ../archon-446-memo-outside/feature.txt";
    let trees = trees(&[("AC-446-002", command, REPO)]);
    let sibling = trees.outside.path().join("archon-446-memo-outside");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join("Cargo.toml"),
        "[package]\nname='outside'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::create_dir_all(sibling.join("src")).unwrap();
    std::fs::write(sibling.join("src/lib.rs"), "pub fn value() {}\n").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = FreezeResume::saving(FreezeBudget::unlimited(), true);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_baseline(super::Baseline {
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        })
        .with_copy_parent(copies.path().to_path_buf())
        .with_resume(&resume);
    let contract = trees.contract();
    let digest = super::contract_digest(&contract).unwrap();
    let refs = super::refs_for(&contract, &digest, &trees.ids());
    let first = probe.run_with_layout(&contract, &digest, &refs).await;
    assert_eq!(first.results.len(), 1);
    assert!(first.results[0].copy_layout.is_some());
    let memo_hit = probe.run_with_layout(&contract, &digest, &refs).await;
    assert_eq!(
        probe.copies_made.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the second observation reused the saved result"
    );
    assert!(memo_hit.results[0].copy_layout.is_some());

    let findings = super::baseline::cannot_fail_findings(
        &probe,
        probe.baseline.as_ref().unwrap(),
        &contract,
        &digest,
        &refs,
        Some(&memo_hit.results),
    )
    .await;
    let unproven = probe.take_unproven();
    assert!(findings.is_empty(), "no author finding: {findings:?}");
    assert!(
        unproven.is_empty(),
        "the copied sibling setup path ran: {unproven:?}"
    );
    trees.assert_live_untouched(copies.path());
}

/// An absent sibling setup path is refused during author review. A missing
/// path inside the check root remains a behavioral result.
#[tokio::test]
async fn an_absent_sibling_setup_path_is_refused_but_an_internal_path_is_not() {
    let outside_copy =
        "cargo metadata --no-deps --format-version 1 --manifest-path ../absent/Cargo.toml";
    let inside_copy =
        "cargo metadata --no-deps --format-version 1 --manifest-path crates/new/Cargo.toml";
    let trees = trees(&[
        ("AC-420-020", outside_copy, REPO),
        ("AC-420-021", inside_copy, REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = super::verdict_probe_tests::round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.contains_key("AC-420-020"), "{findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven.is_empty(),
        "missing sibling paths are refused before a probe: {unproven:?}"
    );
    assert!(findings["AC-420-020"].contains("../absent/Cargo.toml"));
    assert!(!findings.contains_key("AC-420-021"), "{findings:?}");
    trees.assert_live_untouched(copies.path());
}

/// The input mutation in `prove` reruns a Direct baseline check in a fresh
/// hermetic copy. Its output must be judged against that copy's layout too.
#[tokio::test]
async fn an_assignment_prefixed_missing_sibling_setup_path_is_refused() {
    let check = "test -f data/state.txt || cargo metadata --no-deps --format-version 1 --manifest-path ../absent/Cargo.toml";
    let trees = trees(&[(
        "AC-420-022",
        check,
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    )]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.contains_key("AC-420-022"), "{findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven.is_empty(),
        "the author refusal precedes any probe verdict: {unproven:?}"
    );
    trees.assert_live_untouched(copies.path());
}

/// Site layout is a copy-only rule: the same Direct check on the live tree
/// still treats its absent sibling path as an ordinary failed verdict.
#[tokio::test]
async fn a_live_direct_probe_keeps_the_live_tree_setup_path_rule() {
    let check = "cargo metadata --no-deps --format-version 1 --manifest-path ../absent/Cargo.toml";
    let trees = trees(&[(
        "AC-420-023",
        check,
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    )]);
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    );
    let contract = trees.contract();
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(check)
        .current_dir(trees.set.project.path())
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        acceptance_id: "AC-420-023".into(),
        exit_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    };
    let at = probe.check_site(&contract);
    assert!(super::verdict::no_verdict(check, &result, &at).is_none());
    let originals = super::originals(&at, &contract, vec![result]).await;
    assert_eq!(
        originals.get("AC-420-023"),
        Some(&super::Original::Failed),
        "live-tree setup failure remains an ordinary failed check"
    );
}

/// The live tree (an acceptance round's own site, no scratch policy): a
/// project beside its repository runs `--manifest-path ../repo/...`, a real
/// path there. An absent crate is the deliverable not built yet, so the
/// original failed its criterion and is held (A5), never a defect.
#[tokio::test]
async fn on_the_live_tree_a_sibling_repository_path_stays_a_held_failure() {
    let command = "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/new-feature/Cargo.toml";
    let trees = trees(&[("AC-420-010", command, REPO)]);
    let project = trees.outside.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        acceptance_id: "AC-420-010".into(),
        exit_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    };
    let contract = trees.contract();
    let site = HostProbe::at(project, trees.repo.clone(), None).check_site(&contract);
    let originals = super::originals(&site, &contract, vec![result]).await;
    assert_eq!(originals.get("AC-420-010"), Some(&super::Original::Failed));
}

/// R1: a check that names a missing sibling path gets its finding, and every
/// other accepted check in the same call is still probed: none of them is
/// published unprobed.
#[tokio::test]
async fn a_missing_sibling_path_never_lets_the_other_checks_publish_unprobed() {
    let trees = trees(&[
        ("AC-X-001", "test -f ../absent/fixture.txt", REPO),
        ("AC-X-002", "true", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(
        findings
            .get("AC-X-001")
            .is_some_and(|finding| finding.contains("../absent/fixture.txt")),
        "{findings:?}"
    );
    // `true` passes on the pre-implementation tree and names no input, so
    // the probe, once it runs, cannot show it able to fail.
    assert!(
        findings.contains_key("AC-X-002") || unproven.contains_key("AC-X-002"),
        "the other check was never probed: {findings:?}; unproven={unproven:?}"
    );
    trees.assert_live_untouched(copies.path());
}
