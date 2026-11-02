//! Issue 492: a check rejected from its run with inputs moved
//! aside keeps what that run showed -- each input's move state, the inputs
//! moved, the exit status and the check's own (redacted) output -- in the
//! finding its author and the reverify record receive. Each case runs a
//! real check in a real copy.

use super::*;

/// Prove `command` (from the project root) and return its finding. With
/// `bound`, the stored accepted verdict is bound to the current contract.
/// A no-verdict run is first the host's (a strike); like a resume, the
/// proof then runs once more on the same commit, where it is the author's.
async fn proof(command: &str, id: &str, bound: bool) -> (Option<String>, Vec<String>) {
    let trees = trees(&[(id, command, PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let mut refs = super::super::refs_for(&contract, &digest, &trees.ids());
    if !bound {
        refs[0].chain_digest = "another-contract-digest".into();
    }
    let mut seen = Vec::new();
    for _ in 0..2 {
        let probe = round_at(&trees, &trees.base, copies.path());
        let mut findings = super::super::prove::prove(
            &probe,
            &Baseline {
                commit: trees.base.clone(),
                repository: trees.repo.clone(),
            },
            &contract,
            &digest,
            &refs,
            &[id.to_string()],
            &BTreeMap::new(),
        )
        .await;
        seen.extend(probe.take_diagnostics());
        seen.extend(
            probe
                .take_unproven()
                .into_iter()
                .map(|unproven| format!("unproven: {unproven:?}")),
        );
        if let Some(finding) = findings.remove(id) {
            return (Some(finding), seen);
        }
    }
    (None, seen)
}

/// The observation part of `finding`, after its verdict text.
fn observation(finding: &str) -> &str {
    finding
        .split_once("; mutation observation: ")
        .unwrap_or_else(|| panic!("no mutation observation: {finding}"))
        .1
}

fn assert_observed(finding: &str, exit: i32, output: &[&str]) {
    let observed = observation(finding);
    assert!(
        observed.contains("data/state.txt"),
        "the moved input is named: {observed}"
    );
    assert!(
        observed.contains(&format!("mutated_exit_code=Some({exit})")),
        "the exit status of the run with inputs moved: {observed}"
    );
    for line in output {
        assert!(observed.contains(line), "output `{line}` kept: {observed}");
    }
    assert!(
        !observed.contains("archon-mutation-"),
        "protocol lines are facts in the states, not output: {observed}"
    );
}

#[tokio::test]
async fn rejected_mutation_that_still_passes_preserves_move_states_exit_and_output() {
    let command =
        "cat data/state.txt >/dev/null 2>&1; echo observed-stdout; echo observed-stderr >&2; true";
    let (finding, _) = proof(command, "AC-492-001", false).await;
    let finding = finding.expect("a check that still passes is a finding");
    assert!(
        finding.contains("it still passed with data/state.txt moved aside"),
        "{finding}"
    );
    assert_observed(&finding, 0, &["observed-stdout", "observed-stderr"]);
}

#[tokio::test]
async fn rejected_mutation_with_observed_recreation_preserves_its_run() {
    let command = "set -- data/state.txt; grep -q ready \"$1\" || printf ready > \"$1\"; echo recreated-run; true";
    let (finding, _) = proof(command, "AC-492-002", true).await;
    let finding = finding.expect("recreation is a finding");
    assert!(finding.contains("recreated data/state.txt"), "{finding}");
    assert_observed(&finding, 0, &["recreated-run"]);
}

#[tokio::test]
async fn rejected_mutation_with_unknown_reappearance_preserves_its_run() {
    let command = "set -- data/state.txt; grep -q ready \"$1\" || { printf ready > \"$1\"; cat \"$1\" >/dev/null; rm \"$1\"; }; echo reappeared-run; true";
    let (finding, _) = proof(command, "AC-492-003", true).await;
    let finding = finding.expect("an unattributed change is a finding");
    assert!(finding.contains("writer is unknown"), "{finding}");
    assert_observed(&finding, 0, &["reappeared-run"]);
}

#[tokio::test]
async fn rejected_mutation_redacts_before_excerpt() {
    const CANARY: &str = "sk-ant-492-observation-canary-7c2e";
    crate::command::workflow_task_set::passability::test_secret("SERVICE_TOKEN", CANARY);
    // The check text spells the value in two halves, so only the check's
    // output can carry it whole.
    let command = "printf 'token=sk-ant-%s\\n' 492-observation-canary-7c2e; \
                   printf 'auth sk-ant-%s\\n' 492-observation-canary-7c2e >&2; true data/state.txt";
    let (finding, _) = proof(command, "AC-492-005", false).await;
    let finding = finding.expect("a check that still passes is a finding");
    let observed = observation(&finding);
    assert!(!observed.contains(CANARY), "{observed}");
    assert!(observed.contains("REDACTED"), "{observed}");
}
