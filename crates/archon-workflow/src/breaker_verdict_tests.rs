use super::rule::*;
use super::*;
use crate::breakage_proof::*;
use std::collections::BTreeSet;
fn pair() -> PairVerdict {
    PairVerdict {
        task_id: "task".into(),
        id: "item".into(),
        verdict: Verdict::Unmutable,
        run_budget_exhausted: false,
        requirement_text_digest: None,
        mutants: vec![AuthoredMutant {
            mutant_id: "b".into(),
            kind: MutantKind::Behaviour,
            breaks: "keeps the value".into(),
            attempt: 1,
        }],
        attempts: vec![],
        relevance: vec![],
        equivalence: vec![],
        killed_by: BTreeMap::new(),
        patch_keys: BTreeMap::new(),
        owed: None,
        survivors: Vec::new(),
    }
}
fn run(id: &str, kind: MutantKind, outcome: RunOutcome, counted: bool) -> MutantRun {
    MutantRun {
        check_id: "check".into(),
        check_digest: "".into(),
        control_key: "".into(),
        mutant_id: id.into(),
        patch_digest: "".into(),
        task_id: "task".into(),
        requirement_id: "item".into(),
        kind,
        touched: vec![],
        outcome,
        counted,
        not_counted: None,
        evidence: Evidence {
            patch: "".into(),
            output: None,
        },
    }
}
fn identity() -> ModelIdentity {
    ModelIdentity {
        provider_id: Some("local".into()),
        model: "one".into(),
    }
}
#[test]
fn identities_and_quotes_are_exact() {
    let a = identity();
    let mut b = a.clone();
    assert!(!a.distinct(&b));
    b.model = "two".into();
    assert!(a.distinct(&b));
    b = a.clone();
    b.provider_id = Some("other".into());
    assert!(a.distinct(&b));
    for q in [
        "KEEPS the value",
        " keeps\u{2003}the\nvalue ",
        "keeps the value",
    ] {
        assert!(verified_quote("It keeps the value intact.", q));
    }
    for q in ["", "the value", "keeps another value"] {
        assert!(!verified_quote("It keeps the value intact.", q));
    }
}
#[test]
fn supplementary_allocation_and_ambiguity() {
    let mut used = BTreeSet::new();
    for expected in ["SUP-item", "SUP-item-R2", "SUP-item-R3"] {
        let id = mint("item", &mut used).unwrap();
        assert_eq!(id, expected);
        assert_eq!(
            crate::v2::acceptance_stage::coverage::supplementary_requirement(&id),
            Some("item")
        );
    }
    assert_eq!(
        mint("item-R2", &mut used),
        Err("sup_id_ambiguous: item-R2".into())
    );
}
#[test]
fn verdict_precedence_table() {
    let p = pair();
    let decide_at =
        |p: &PairVerdict, r: &[MutantRun], s: &[ProofState], refused, text, distinct| {
            decide(Decision {
                pair: p,
                runs: r,
                states: s,
                refused,
                has_text: text,
                distinct,
            })
        };
    assert_eq!(decide_at(&p, &[], &[], true, false, true), Verdict::Refused);
    for (s, v) in [
        (ProofState::NoPassingCheck, Verdict::NoPassingCheck),
        (
            ProofState::NoCountableDeliverable,
            Verdict::NoCountableDeliverable,
        ),
        (ProofState::BudgetExhausted, Verdict::BudgetExhausted),
        (ProofState::HostUnrun, Verdict::HostUnrun),
        (ProofState::Unverified, Verdict::Unverified),
    ] {
        assert_eq!(decide_at(&p, &[], &[s], false, true, true), v);
    }
    assert_eq!(
        decide_at(&p, &[], &[], false, false, true),
        Verdict::RequirementTextMissing
    );
    let mut p = p;
    p.mutants.clear();
    assert_eq!(
        decide_at(&p, &[], &[], false, true, true),
        Verdict::BreakerUnavailable
    );
    p = pair();
    let kill = run(
        "b",
        MutantKind::Behaviour,
        RunOutcome::Killed {
            exit_code: 1,
            elapsed_ms: 1,
        },
        true,
    );
    assert_eq!(
        decide_at(&p, std::slice::from_ref(&kill), &[], false, true, true),
        Verdict::UndecidedRelevance
    );
    p.relevance.push(RelevanceMark {
        mutant_id: "b".into(),
        check_id: "check".into(),
        relevance: Relevance::Relevant,
        quote: Some("keeps the value".into()),
        reason: None,
        judge: identity(),
        cause: None,
    });
    assert_eq!(
        decide_at(&p, std::slice::from_ref(&kill), &[], false, true, true),
        Verdict::BodyGutUnavailable
    );
    let gut = run(
        "g",
        MutantKind::BodyGut,
        RunOutcome::Killed {
            exit_code: 1,
            elapsed_ms: 1,
        },
        true,
    );
    assert_eq!(
        decide_at(&p, &[kill.clone(), gut.clone()], &[], false, true, true),
        Verdict::DoneCandidate
    );
    let survives = run(
        "b",
        MutantKind::Behaviour,
        RunOutcome::Survived { elapsed_ms: 1 },
        false,
    );
    assert_eq!(
        decide_at(&p, std::slice::from_ref(&survives), &[], false, true, false),
        Verdict::JudgeUnavailable
    );
    assert_eq!(
        decide_at(&p, std::slice::from_ref(&survives), &[], false, true, true),
        Verdict::JudgeUnavailable
    );
    let answer = JudgeAnswer {
        judge: identity(),
        answer: EquivalenceAnswer::Unsure,
        reason: None,
        cause: None,
    };
    p.equivalence.push(EquivalenceMark {
        mutant_id: "b".into(),
        first: answer.clone(),
        second: answer,
        marked: false,
        capped: false,
    });
    assert_eq!(
        decide_at(
            &p,
            &[survives],
            &[ProofState::BudgetExhausted],
            false,
            true,
            true
        ),
        Verdict::WeakTest
    );
    assert_eq!(
        decide_at(&pair(), &[], &[], false, true, true),
        Verdict::HostUnrun
    );
    let invalid = run(
        "b",
        MutantKind::Behaviour,
        RunOutcome::Unviable { located: vec![] },
        false,
    );
    assert_eq!(
        decide_at(&pair(), &[invalid], &[], false, true, true),
        Verdict::NoViableBehaviourMutant
    );
    p.relevance[0].relevance = Relevance::Unavailable;
    assert_eq!(
        decide_at(&p, &[kill, gut], &[], false, true, true),
        Verdict::JudgeUnavailable
    );
}

#[test]
fn requirement_text_lookup_and_unchanged_lock_bytes() {
    let text = "## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Keeps the good value. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Displays the retained value. |\n";
    assert!(
        requirement_text(text, "REQ-X-001")
            .unwrap()
            .contains("Keeps the good value")
    );
    assert!(
        requirement_text(text, "AC-X-001")
            .unwrap()
            .contains("Displays the retained value")
    );
    assert_eq!(
        requirement_text(text, "SUP-REQ-X-001"),
        requirement_text(text, "REQ-X-001")
    );
    assert_eq!(requirement_text(text, "absent"), None);
    for name in ["acceptance.lock", "skeleton.lock"] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/classification")
            .join(name);
        let bytes = std::fs::read(&source).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

fn verify_patch_name(key: &str) {
    let root = tempfile::tempdir().unwrap();
    let invoke = |args: &[&str]| {
        let output = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    invoke(&["init", "-q"]);
    invoke(&["config", "core.autocrlf", "false"]);
    let source = root.path().join(crate::base_snapshot::key_path(key));
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, b"good\n").unwrap();
    std::fs::write(root.path().join("other"), b"untouched").unwrap();
    let patch = edit_patch(key, "good\n", "bad\n");
    assert_eq!(patch.lines().count(), 6);
    let path = root.path().join("change");
    std::fs::write(&path, patch).unwrap();
    invoke(&["apply", "--check", "--", path.to_str().unwrap()]);
    invoke(&["apply", "--", path.to_str().unwrap()]);
    assert_eq!(std::fs::read(source).unwrap(), b"bad\n");
    assert_eq!(
        std::fs::read(root.path().join("other")).unwrap(),
        b"untouched"
    );
}
#[test]
fn patch_names_preserve_spaces_unicode_and_nested_keys() {
    for key in ["plain", "with space", "雪", "nested/with space"] {
        verify_patch_name(key);
    }
}
#[cfg(unix)]
#[test]
fn patch_names_cannot_inject_header_lines() {
    for key in ["line\nname", "tab\tname", "quote\"name"] {
        verify_patch_name(key);
    }
}

#[test]
fn budget_stops_and_unrun_mutants_cannot_be_done() {
    let mut p = pair();
    p.mutants.push(AuthoredMutant {
        mutant_id: "g".into(),
        kind: MutantKind::BodyGut,
        breaks: String::new(),
        attempt: 1,
    });
    p.mutants.push(AuthoredMutant {
        mutant_id: "unrun".into(),
        kind: MutantKind::Behaviour,
        breaks: "keeps the value".into(),
        attempt: 1,
    });
    p.relevance.push(RelevanceMark {
        mutant_id: "b".into(),
        check_id: "check".into(),
        relevance: Relevance::Relevant,
        quote: Some("keeps the value".into()),
        reason: None,
        judge: identity(),
        cause: None,
    });
    let mut runs = vec![
        run(
            "b",
            MutantKind::Behaviour,
            RunOutcome::Killed {
                exit_code: 1,
                elapsed_ms: 1,
            },
            true,
        ),
        run(
            "g",
            MutantKind::BodyGut,
            RunOutcome::Killed {
                exit_code: 1,
                elapsed_ms: 1,
            },
            true,
        ),
    ];
    for states in [
        vec![],
        vec![ProofState::BehaviourKill],
        vec![ProofState::BudgetExhausted],
    ] {
        runs.truncate(2);
        runs.push(run(
            "unrun",
            MutantKind::Behaviour,
            RunOutcome::BudgetExhausted,
            false,
        ));
        assert_eq!(
            decide(Decision {
                pair: &p,
                runs: &runs,
                states: &states,
                refused: false,
                has_text: true,
                distinct: true
            }),
            Verdict::BudgetExhausted
        );
    }
}
#[test]
fn unviable_mutants_are_not_survivors_even_with_a_passing_check() {
    let runs = [
        run(
            "b",
            MutantKind::Behaviour,
            RunOutcome::Unviable { located: vec![] },
            false,
        ),
        run(
            "b",
            MutantKind::Behaviour,
            RunOutcome::Survived { elapsed_ms: 1 },
            false,
        ),
    ];
    assert!(!survivor("b", &runs));
}

#[test]
fn real_body_survivor_wins_over_budget_and_missing_gut_is_listed() {
    let mut p = pair();
    p.mutants[0].kind = MutantKind::BodyGut;
    let runs = [run(
        "b",
        MutantKind::BodyGut,
        RunOutcome::Survived { elapsed_ms: 1 },
        false,
    )];
    assert_eq!(
        decide(Decision {
            pair: &p,
            runs: &runs,
            states: &[ProofState::BudgetExhausted],
            refused: false,
            has_text: true,
            distinct: false
        }),
        Verdict::WeakTest
    );
}

#[test]
fn body_author_outage_is_transient_after_behaviour_authoring() {
    let mut pair = pair();
    pair.attempts.push(AttemptRecord {
        attempt: 1,
        reply_key: "body_gut:reply".into(),
        outcome: AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            reason: "provider unavailable".into(),
        },
        refusals: vec![],
    });
    let runs = [run(
        "b",
        MutantKind::Behaviour,
        RunOutcome::Unviable { located: vec![] },
        false,
    )];
    assert_eq!(
        decide(Decision {
            refused: false,
            has_text: true,
            states: &[],
            pair: &pair,
            runs: &runs,
            distinct: false
        }),
        Verdict::BreakerUnavailable
    );
}

#[test]
fn requirement_span_extracts_longest_original_words() {
    assert_eq!(
        requirement_span(
            "It KEEPS the good value for every input.",
            "because it keeps THE good value for every input. and explains why"
        ),
        Some("It KEEPS the good value for every input.".into())
    );
    assert_eq!(
        requirement_span(
            "one two three four five",
            "one two three then two three four five"
        ),
        Some("two three four five".into())
    );
    assert_eq!(requirement_span("one two three", "someone two three"), None);
    assert_eq!(requirement_span("one two", "one two"), None);
    assert_eq!(
        requirement_span("One\nTwo\tTHREE", "one two three"),
        Some("One Two THREE".into())
    );
}

#[test]
fn quotes_spanning_standalone_punctuation_keep_the_whole_span() {
    assert!(!verified_quote(
        "Coverage must be >= 80 percent",
        "Coverage must be <= 80 percent"
    ));
    for text in [
        "Coverage must be >= 80 percent",
        "Coverage must be - 80 percent",
        "Coverage must be -> 80 percent",
        "Coverage must be = 80 percent",
    ] {
        assert!(verified_quote(text, text));
        assert_eq!(requirement_span(text, text).as_deref(), Some(text));
    }
}
