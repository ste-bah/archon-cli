//! Regression proof cases for stored judgment binding and fallback behavior.

use super::*;
use archon_workflow::task_set_contract::JudgeDecision;

#[tokio::test]
async fn a_bound_accepted_judgment_does_not_exempt_a_vacuous_check() {
    let trees = trees(&[("AC-3B-001", "echo ok", PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let mut contract = trees.contract();
    let ids = trees.ids();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &ids);
    let passing = vec!["AC-3B-001".to_string()];
    let accepted = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &refs,
        &passing,
    )
    .await;
    assert!(
        accepted.get("AC-3B-001").is_some_and(
            |finding| finding.contains("no movable input exists under the selected root")
        ),
        "a stored verdict cannot make `echo ok` prove its criterion: {accepted:?}"
    );
    let diagnostics = probe.take_diagnostics();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    contract.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
    let rejected_digest = super::super::contract_digest(&contract).unwrap();
    let mut rejected_ref = refs[0].clone();
    rejected_ref.chain_digest = rejected_digest.clone();
    let rejected = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &rejected_digest,
        &[rejected_ref],
        &passing,
    )
    .await;
    assert!(rejected.contains_key("AC-3B-001"), "{rejected:?}");

    let mut stale_ref = refs[0].clone();
    stale_ref.chain_digest = "stale-contract-digest".into();
    let stale = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &trees.contract(),
        &digest,
        &[stale_ref],
        &passing,
    )
    .await;
    assert!(stale.contains_key("AC-3B-001"), "{stale:?}");
}

async fn bound_proof(command: &str, id: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let trees = trees(&[(id, command, PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let findings = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &refs,
        &[id.to_string()],
    )
    .await;
    (findings, probe.take_diagnostics())
}

#[tokio::test]
async fn a_bound_check_that_recreates_reads_and_deletes_the_input_is_a_finding() {
    let command = "set -- data/state.txt; grep -q ready \"$1\" || { printf ready > \"$1\"; cat \"$1\" >/dev/null; rm \"$1\"; }; true";
    let (findings, diagnostics) = bound_proof(command, "AC-3B-006").await;
    assert!(
        findings
            .get("AC-3B-006")
            .is_some_and(|finding| finding.contains("recreated data/state.txt")),
        "create-read-delete must invalidate the stored verdict: {findings:?}; {diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|note| note.contains("accepted as a regression guard")),
        "{diagnostics:?}"
    );
}

#[tokio::test]
async fn a_bound_check_that_recreates_and_leaves_the_input_is_a_finding() {
    let command = "set -- data/state.txt; grep -q ready \"$1\" || printf ready > \"$1\"; true";
    let (findings, diagnostics) = bound_proof(command, "AC-3B-007").await;
    assert!(
        findings
            .get("AC-3B-007")
            .is_some_and(|finding| finding.contains("recreated data/state.txt")),
        "a generated-path marker cannot hide recreation: {findings:?}; {diagnostics:?}"
    );
}

#[tokio::test]
async fn an_honest_bound_check_that_leaves_the_moved_path_missing_is_accepted() {
    let trees = trees(&[("AC-3B-008", "true data/state.txt", PROJECT)]);
    std::fs::write(trees.set.project.path().join("data/other.txt"), "untouched").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let findings = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &refs,
        &["AC-3B-008".into()],
    )
    .await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(
        probe
            .take_diagnostics()
            .iter()
            .any(|note| note.contains("accepted as a regression guard"))
    );
    assert_eq!(
        std::fs::read_to_string(trees.set.project.path().join("data/other.txt")).unwrap(),
        "untouched"
    );
}

#[tokio::test]
async fn an_unrelated_temporary_file_elsewhere_does_not_invalidate_a_bound_check() {
    let command = "tmp=$(mktemp) || exit 1; printf x > \"$tmp\"; rm \"$tmp\"; true data/state.txt";
    let (findings, diagnostics) = bound_proof(command, "AC-3B-009").await;
    assert!(findings.is_empty(), "{findings:?}; {diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .any(|note| note.contains("accepted as a regression guard")),
        "{diagnostics:?}"
    );
}

#[tokio::test]
async fn a_real_baseline_passing_check_is_proven_by_its_input() {
    let trees = trees(&[("AC-3B-005", "grep -q ready data/state.txt", PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let contract = trees.contract();
    let findings = probe.script_defects(&contract, &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    let diagnostics = probe.take_diagnostics();
    assert!(
        diagnostics
            .iter()
            .any(|note| note.contains("proven able to fail") && note.contains("data/state.txt")),
        "{diagnostics:?}"
    );
}

#[tokio::test]
async fn missing_judgments_do_not_exempt_vacuous_or_self_filled_checks() {
    let trees = trees(&[
        ("AC-3B-002", "true", PROJECT),
        ("AC-3B-003", "echo ok", PROJECT),
        (
            "AC-3B-004",
            "printf 'ready\\n' > generated.txt && grep -q ready generated.txt",
            PROJECT,
        ),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let passing = vec!["AC-3B-002".into(), "AC-3B-003".into(), "AC-3B-004".into()];
    let findings = super::super::prove::prove(
        &probe,
        &Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &[],
        &passing,
    )
    .await;
    assert_eq!(findings.keys().cloned().collect::<Vec<_>>(), passing);
}
