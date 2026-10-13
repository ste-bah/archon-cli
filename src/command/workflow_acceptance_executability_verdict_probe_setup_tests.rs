//! Issue 420 (gap 2) through a real freeze probe on real trees: a check
//! whose pre-implementation failure is a setup path outside the probe's
//! copy (a sibling root the copy does not hold) is unproven with that
//! cause, never a baseline failure the passability judge is shown; a check
//! failing on an absent deliverable inside the tree is still a proof.

use super::probe_tests::trees;
use super::verdict_probe_tests::{REPO, round_at};
use super::{ExecutabilityProbe, HostProbe};

#[tokio::test]
async fn a_sibling_root_absent_from_the_copy_is_unproven_and_never_judged() {
    let sibling = "cargo metadata --no-deps --format-version 1 --manifest-path ../archon-420-code-root/Cargo.toml";
    let enters = "cd ../archon-420-code-root && test -f feature.txt";
    let deliverable = "cargo metadata --no-deps --format-version 1 --manifest-path crates/archon-420-new/Cargo.toml";
    // `../repo` is the repository copy the probe's copy holds: a deliverable.
    let held_root = "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/archon-420-new/Cargo.toml";
    let trees = trees(&[
        ("AC-420-001", sibling, REPO),
        ("AC-420-002", enters, REPO),
        ("AC-420-003", deliverable, REPO),
        ("AC-420-004", held_root, REPO),
    ]);
    // The sibling root exists beside the live repository, so the live
    // layout would find it; the probe's copy holds only the repository and
    // the project.
    let live_sibling = trees.outside.path().join("archon-420-code-root");
    std::fs::create_dir_all(&live_sibling).unwrap();
    std::fs::write(live_sibling.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(
        findings.is_empty(),
        "never the author's on the first strike: {findings:?}"
    );
    for (id, path) in [
        ("AC-420-001", "../archon-420-code-root/Cargo.toml"),
        ("AC-420-002", "../archon-420-code-root"),
    ] {
        let why = unproven.get(id).unwrap_or_else(|| {
            panic!(
                "{id}: a setup failure counted as a proof: {unproven:?} {:?}",
                probe.take_diagnostics()
            )
        });
        assert!(why.contains("outside the tree"), "{id}: {why}");
        assert!(why.contains(path), "{id}: {why}");
        assert!(why.contains(&trees.base[..12]), "{id}: {why}");
    }
    assert!(!unproven.contains_key("AC-420-003"), "{unproven:?}");
    assert!(!unproven.contains_key("AC-420-004"), "{unproven:?}");
    let runs = probe.take_baseline_runs().expect("a baseline");
    let judged: Vec<&String> = runs.failures.keys().collect();
    assert_eq!(
        judged,
        vec!["AC-420-003", "AC-420-004"],
        "only the proofs reach passability"
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
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_baseline(super::Baseline {
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        })
        .with_copy_parent(copies.path().to_path_buf());

    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(findings.is_empty(), "no author finding: {findings:?}");
    let why = unproven
        .get("AC-446-001")
        .expect("outside-copy setup failure is unproven");
    assert!(why.contains("outside the tree"), "{why}");
    assert!(why.contains("../archon-446-outside/Cargo.toml"), "{why}");
    let runs = probe.take_baseline_runs().expect("a baseline");
    assert!(runs.failures.is_empty(), "no baseline verdict: {runs:?}");
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_hermetic_memo_hit_keeps_its_layout_for_judgment() {
    use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

    let command = "cargo metadata --no-deps --format-version 1 --manifest-path ../archon-446-memo-outside/Cargo.toml";
    let trees = trees(&[("AC-446-002", command, REPO)]);
    let sibling = trees.outside.path().join("archon-446-memo-outside");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join("Cargo.toml"),
        "[package]\nname='outside'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
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
    let why = unproven
        .get("AC-446-002")
        .expect("memoized outside-copy setup failure is unproven");
    assert!(why.contains("outside the tree"), "{why}");
    assert!(
        why.contains("../archon-446-memo-outside/Cargo.toml"),
        "{why}"
    );
    trees.assert_live_untouched(copies.path());
}

/// A round's direct site executes baseline checks in a hermetic copy too;
/// its verdict context must describe that copy, while the same check on the
/// live tree keeps the live-tree rule.
#[tokio::test]
async fn a_direct_baseline_uses_its_hermetic_copy_layout_for_setup_paths() {
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
    assert!(findings.is_empty(), "{findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven.contains_key("AC-420-020"),
        "the baseline copy's absent outside path is unproven: {unproven:?}"
    );
    assert!(
        !unproven.contains_key("AC-420-021"),
        "an absent path inside the copy remains a verdict: {unproven:?}"
    );
    let runs = probe.take_baseline_runs().expect("a baseline");
    assert_eq!(
        runs.failures.keys().cloned().collect::<Vec<_>>(),
        ["AC-420-021"]
    );

    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(outside_copy)
        .current_dir(&trees.repo)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-420-020".into(),
        exit_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    };
    let contract = trees.contract();
    let live = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .check_site(&contract);
    let originals = super::originals(&live, &contract, vec![result]).await;
    assert_eq!(
        originals.get("AC-420-020"),
        Some(&super::Original::Failed),
        "live tree paths stay outside the setup-path rule"
    );
    trees.assert_live_untouched(copies.path());
}

/// The input mutation in `prove` reruns a Direct baseline check in a fresh
/// hermetic copy. Its output must be judged against that copy's layout too.
#[tokio::test]
async fn a_direct_prove_rerun_with_a_missing_outside_setup_path_is_unproven() {
    let check = "test -f data/state.txt || cargo metadata --no-deps --format-version 1 --manifest-path ../absent/Cargo.toml";
    let trees = trees(&[(
        "AC-420-022",
        check,
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    )]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "no author finding: {findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven.contains_key("AC-420-022"),
        "the setup failure in the prove rerun has no verdict: {unproven:?}"
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
