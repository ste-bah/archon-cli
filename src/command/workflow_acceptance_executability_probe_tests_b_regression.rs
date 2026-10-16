//! Regression proof cases for stored judgment binding and fallback behavior.

use super::*;
use archon_workflow::task_set_contract::JudgeDecision;

#[tokio::test]
async fn baseline_pass_uses_only_an_accepted_judgment_bound_to_the_frozen_check() {
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
    assert!(accepted.is_empty(), "{accepted:?}");
    let diagnostics = probe.take_diagnostics();
    assert!(
        diagnostics.iter().any(|note| note
            .contains("accepted as a regression guard on its stored accepted judge verdict")),
        "{diagnostics:?}"
    );

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

#[tokio::test]
async fn a_real_baseline_passing_check_is_recorded_on_its_bound_judgment() {
    let trees = trees(&[("AC-3B-005", "grep -q ready data/state.txt", PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let contract = trees.contract();
    let findings = probe.script_defects(&contract, &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    let diagnostics = probe.take_diagnostics();
    assert!(
        diagnostics.iter().any(|note| note
            .contains("accepted as a regression guard on its stored accepted judge verdict")),
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
