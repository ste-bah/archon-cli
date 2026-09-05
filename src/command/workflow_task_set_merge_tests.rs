use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, GapPolicy, JudgeVerdict, PrdIdentity, TrustedCwd,
};

fn criterion(id: &str, command: &str, verdict: JudgeDecision) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion {id}"),
        check: AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::ProjectRoot,
        },
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict,
            counterexample: "c".into(),
            reason: "r".into(),
            host_call_id: "j".into(),
            sampling: None,
        },
    }
}

fn contract(digest: &str, entries: Vec<AcceptanceCriterion>) -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "p".into(),
            digest: digest.into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: entries,
        supplementary: Vec::new(),
    }
}

/// A refuted check whose id the base holds accepted takes the base entry,
/// verdict included; an accepted check in the new candidate is untouched.
#[test]
fn a_refuted_check_takes_the_previously_accepted_entry() {
    let base = contract(
        "d",
        vec![
            criterion("AC-1", "test -f a", JudgeDecision::Accepted),
            criterion("AC-2", "test -f b", JudgeDecision::Refuted),
        ],
    );
    let mut new = contract(
        "d",
        vec![
            criterion("AC-1", "true", JudgeDecision::Refuted),
            criterion("AC-2", "test -f b && grep x b", JudgeDecision::Accepted),
        ],
    );
    let replaced = keep_previously_accepted(&mut new, &base);
    assert_eq!(replaced, vec!["AC-1".to_string()]);
    assert!(
        matches!(&new.acceptance[0].check, AcceptanceCheck::Command { command, .. } if command == "test -f a")
    );
    assert_eq!(new.acceptance[0].judgment.verdict, JudgeDecision::Accepted);
    assert!(
        matches!(&new.acceptance[1].check, AcceptanceCheck::Command { command, .. } if command.contains("grep"))
    );
}

/// Nothing crosses a PRD boundary, a changed criterion text, or a base entry
/// that was itself defective.
#[test]
fn nothing_is_kept_across_a_different_prd_criterion_or_defective_base() {
    let base = contract(
        "d",
        vec![criterion("AC-1", "test -f a", JudgeDecision::Accepted)],
    );
    let mut other_prd = contract("e", vec![criterion("AC-1", "true", JudgeDecision::Refuted)]);
    assert!(keep_previously_accepted(&mut other_prd, &base).is_empty());

    let mut renamed = contract("d", vec![criterion("AC-1", "true", JudgeDecision::Refuted)]);
    renamed.acceptance[0].criterion = "a different sentence".into();
    assert!(keep_previously_accepted(&mut renamed, &base).is_empty());

    let mut gap_changed = contract("d", vec![criterion("AC-1", "true", JudgeDecision::Refuted)]);
    gap_changed.acceptance[0].gap_permitted = true;
    gap_changed
        .gap_policy
        .permitted_acceptance_ids
        .insert("AC-1".into());
    assert!(keep_previously_accepted(&mut gap_changed, &base).is_empty());

    let refuted_base = contract(
        "d",
        vec![criterion("AC-1", "test -f a", JudgeDecision::Refuted)],
    );
    let mut new = contract("d", vec![criterion("AC-1", "true", JudgeDecision::Refuted)]);
    assert!(keep_previously_accepted(&mut new, &refuted_base).is_empty());
}
