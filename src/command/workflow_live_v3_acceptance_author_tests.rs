//! REM-13 / ACC-A7: the host's in-round authoring stages what it authored
//! and owes the rest by name.

use super::*;

#[test]
fn a_placeholder_is_never_publishable_and_owed_checks_are_named_until_authored() {
    let entry = placeholder("AC-1", "the gate holds", Vec::new());
    assert_eq!(entry.judgment.verdict, JudgeDecision::Refuted);
    assert!(crate::command::workflow_task_set::executability::is_placeholder(&entry));
    let dir = tempfile::tempdir().unwrap();
    let mut staged = Staged::load(dir.path(), "digest-1").unwrap();
    let owed = vec![
        placeholder("AC-1", "the gate holds", Vec::new()),
        placeholder(
            "SUP-REQ-1",
            "the store is append-only",
            vec!["REQ-1".into()],
        ),
    ];
    staged.reject("AC-1", "the judge refuted it");
    let errors = owed_errors(&owed, &staged, "test");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("AC-1") && errors[0].contains("the judge refuted it"));
    let mut accepted = placeholder("AC-1", "the gate holds", Vec::new());
    accepted.judgment.verdict = JudgeDecision::Accepted;
    staged.entries.insert("AC-1".into(), accepted);
    staged.save(dir.path()).unwrap();
    // Kept across rounds for the same PRD; dropped when the PRD moved.
    let again = Staged::load(dir.path(), "digest-1").unwrap();
    assert_eq!(owed_errors(&owed, &again, "test").len(), 1);
    assert!(
        Staged::load(dir.path(), "digest-2")
            .unwrap()
            .entries
            .is_empty()
    );
    // Fail closed (7-9): staging that exists but cannot be read is an
    // error, never silently dropped work.
    std::fs::write(staging_path(dir.path()), b"not staging").unwrap();
    assert!(
        Staged::load(dir.path(), "digest-1")
            .unwrap_err()
            .contains("unreadable")
    );
    // Supplementary entries land in `supplementary`, sorted.
    let base = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prd.md".into(),
            digest: "digest-1".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: Vec::new(),
        supplementary: Vec::new(),
    };
    let contract = working(&base, &owed, &again);
    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(contract.supplementary[0].id, "SUP-REQ-1");
    assert_eq!(contract.supplementary[0].covers, ["REQ-1"]);
}

#[test]
fn changed_requirement_text_makes_its_frozen_supplementary_entry_drift() {
    let previous = placeholder(
        "SUP-REQ-X-001",
        "The dataset records provider and instrument.",
        vec!["REQ-X-001".into()],
    );
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prd.md".into(),
            digest: "before-change".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: Vec::new(),
        supplementary: vec![previous],
    };
    let owed = super::super::author_drift::owed_by(
        &contract,
        "## Requirements\n\n- REQ-X-001: The dataset records provider, instrument, timeframe, and price basis.\n",
        true,
    );
    assert_eq!(owed.len(), 1, "the frozen entry is re-authored in place");
    assert_eq!(owed[0].id, "SUP-REQ-X-001");
    assert_eq!(
        owed[0].criterion,
        "The dataset records provider, instrument, timeframe, and price basis."
    );
}

#[test]
fn r7_staged_feedback_keeps_every_closing_fence_within_the_cap() {
    let mut staged = Staged::default();
    let why = format!(
        "{}[begin untrusted program output]\n{}\n[end untrusted program output]\nstdout:\n[begin untrusted program output]\n{}\n[end untrusted program output]",
        "reason ".repeat(270),
        "界".repeat(3000),
        "b".repeat(1000)
    );
    staged.reject("AC-1", &why);
    let feedback = &staged.feedback["AC-1"];
    assert_eq!(
        feedback.matches("[begin untrusted program output]").count(),
        feedback.matches("[end untrusted program output]").count(),
        "{feedback}"
    );
    assert!(feedback.chars().count() <= 4000);
}
