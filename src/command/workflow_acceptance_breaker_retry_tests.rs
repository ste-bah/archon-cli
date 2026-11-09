use super::tests::{fixture, request};
use super::*;
use test_client::{Client, corpus};
#[tokio::test]
async fn unviable_retry_uses_remainder_and_confined_feedback() {
    let command = "case \"$(sh v*)\" in good) exit 0;; bad|wrong|'') printf 'value:3:5: error invalid\\n'; exit 1;; *) exit 1;; esac";
    let f = fixture(command);
    let p = f.probe(false);
    let c = Client::default();
    let r = prove_requirements(&p, &c, request(&f, 10)).await;
    assert_eq!(r.verdicts.rounds, 2);
    assert!(r.verdicts.runs_executed <= 10);
    // All behaviour proposals are unviable, so the pair is listed even when the gut also fails.
    assert_eq!(
        r.verdicts.entries[0].verdict,
        Verdict::NoViableBehaviourMutant
    );
    assert!(
        r.verdicts.entries[0]
            .attempts
            .iter()
            .flat_map(|a| &a.refusals)
            .any(|r| r == "value:3")
    );
    let calls = c.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.task.contains("Pick one function or block"))
            .count(),
        2
    );
    let feedback = &calls[2].task;
    for forbidden in ["killed", "survived", "check", "value:3:5: error invalid"] {
        assert!(!feedback.contains(forbidden), "{feedback}");
    }
    assert!(feedback.contains("value:3"));
    drop(calls);
    // Fresh host-supplied patches test the viable retry path without inventing provider replies.
    let mutants = [
        (MutantKind::Behaviour, "changed"),
        (MutantKind::BodyGut, "minimal"),
    ]
    .into_iter()
    .map(|(kind, value)| SuppliedMutant {
        task_id: "task".into(),
        requirement_id: "REQ-VALUE-001".into(),
        kind,
        patch: rule::edit_patch(
            "value",
            std::str::from_utf8(super::tests::VALUE).unwrap(),
            &format!("value() {{ printf {value}; }}\nvalue\n"),
        ),
    })
    .collect();
    let set = MutantSet {
        schema: BREAKAGE_SCHEMA,
        base_commit: f.lock.baseline_commit.clone().unwrap(),
        mutants,
    };
    let report = prove_breakage(
        &p,
        BreakageRequest {
            contract: &f.contract,
            chain_digest: &f.lock.digest,
            lock: &f.lock,
            classification: &f.classes,
            mutants: &set,
            max_runs: 10,
            min_deadline_secs: 2,
        },
    )
    .await;
    let runs: Vec<_> = report.proofs.runs.values().collect();
    assert!(
        runs.iter()
            .any(|r| r.counted && r.kind == MutantKind::Behaviour)
    );
    assert!(
        runs.iter()
            .any(|r| r.counted && r.kind == MutantKind::BodyGut)
    );
    assert!(
        runs.iter()
            .all(|r| matches!(r.outcome, RunOutcome::Killed { .. }))
    );
    let mut pair = r.verdicts.entries[0].clone();
    pair.mutants = set
        .mutants
        .iter()
        .map(|m| AuthoredMutant {
            mutant_id: m.mutant_id(),
            kind: m.kind,
            breaks: "keeps the good value".into(),
            attempt: 2,
        })
        .collect();
    pair.relevance.clear();
    pair.equivalence.clear();
    let req = request(&f, 10);
    let circuit = call::Circuit::new(20, &req.call_limits);
    let mut ctx = Context {
        pending: vec![],
        probe: &p,
        client: &c,
        req,
        verdicts: r.verdicts,
        store: None,
        circuit,
    };
    let view = super::view(&ctx, 0).await.unwrap();
    let runs: Vec<_> = runs.into_iter().cloned().collect();
    judges::relevance(&mut ctx, &mut pair, &view, &runs, None, super::tests::TEXT).await;
    assert_eq!(
        rule::decide(rule::Decision {
            refused: false,
            has_text: true,
            states: &[],
            pair: &pair,
            runs: &runs,
            distinct: true
        }),
        Verdict::DoneCandidate
    );
}
#[tokio::test]
async fn named_deliverable_remains_in_view_and_refusal_prevents_calls() {
    let f = fixture("test -f value");
    let p = f.probe(false);
    let c = Client::default();
    let r = prove_requirements(&p, &c, request(&f, 10)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::WeakTest);
    assert_eq!(c.calls.lock().unwrap().len(), 2);
    assert!(r.verdicts.entries[0].owed.is_some());
    let c = Client::default();
    let f = fixture("test -s v*");
    let mut p = f.probe(false);
    p.site = Site::Direct;
    let r = prove_requirements(&p, &c, request(&f, 10)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::Refused);
    assert_eq!(r.verdicts.runs_executed, 0);
    assert!(c.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn refused_mutants_retry_and_unusable_replies_get_format_feedback() {
    let f = fixture("test -d .");
    let p = f.probe(false);
    let mut c = Client::default();
    c.answers
        .insert("breaker".into(), corpus("breaker_r18_attempt1"));
    c.answers.insert("breaker_gut".into(), corpus("relevant"));
    let r = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::BreakerUnusable);
    assert_eq!(c.calls.lock().unwrap().len(), 4);
    assert!(
        c.calls.lock().unwrap()[3]
            .task
            .contains("breaks_not_in_requirement")
    );
    assert!(
        c.calls.lock().unwrap()[1]
            .task
            .contains("keeps the good value")
    );
    assert_eq!(r.verdicts.rounds, 0);
    let mut c = Client::default();
    c.answers.insert("breaker".into(), corpus("relevant"));
    c.answers.insert("breaker_gut".into(), corpus("relevant"));
    let r = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::BreakerUnusable);
    let calls = c.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(
        calls[3]
            .task
            .contains("Previous reply unusable: invalid_reply")
    );
}

#[tokio::test]
async fn retry_does_not_replay_an_older_positive_breaker_reply() {
    let mut f = fixture("sh v*");
    let p = f.probe(true);
    let mut c = Client::default();
    c.answers
        .insert("not_equivalent".into(), corpus("equivalent"));
    let first = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::WeakTest);
    f.contract.acceptance[0].check=serde_json::from_value(serde_json::json!({"kind":"command","command":"observed=$(sh v*); if test \"$observed\" != good; then printf 'wrong value: expected good, got %s\\n' \"$observed\"; exit 1; fi","cwd":"repo_root"})).unwrap();
    f.lock.digest = content_digest(b"stronger");
    f.classes.acceptance_digest = f.lock.digest.clone();
    f.lock.baseline_results = archon_workflow::baseline_results::BaselineResults::observed(
        f.lock.baseline_commit.clone(),
        &[(
            "check".into(),
            archon_workflow::baseline_results::BaselineOutcome::Passed,
        )]
        .into(),
        &f.contract,
    )
    .outcomes;
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    let c = Client::default();
    let report = prove_requirements(&p, &c, request(&f, 20)).await;
    assert!(report.verdicts.entries[0].survivors.is_empty());
    assert!(c.calls.lock().unwrap().is_empty());
    assert!(
        report.verdicts.entries[0]
            .killed_by
            .values()
            .all(|ids| ids.contains(&"check".into()))
    );
    assert_eq!(report.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert_eq!(report.verdicts.rounds, 1);
    let reused = Client::default();
    let again = prove_requirements(&p, &reused, request(&f, 0)).await;
    assert_eq!(again.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert_eq!(again.verdicts.runs_executed, 0);
    assert!(reused.calls.lock().unwrap().is_empty());
    assert!(reused.completions.lock().unwrap().is_empty());
}

#[test]
fn repeated_patch_preserves_body_removal_identity() {
    let mut pair = serde_json::from_value::<PairVerdict>(
        serde_json::json!({"task_id":"task","id":"item","verdict":"unmutable","killed_by":{}}),
    )
    .unwrap();
    for (kind, expected) in [
        (MutantKind::Behaviour, MutantKind::Behaviour),
        (MutantKind::BodyGut, MutantKind::BodyGut),
        (MutantKind::Behaviour, MutantKind::BodyGut),
    ] {
        author::record_authored(
            &mut pair,
            AuthoredMutant {
                mutant_id: "same".into(),
                kind,
                breaks: String::new(),
                attempt: 2,
            },
        );
        assert_eq!(pair.mutants.len(), 1);
        assert_eq!(pair.mutants[0].kind, expected);
    }
    assert!(rule::real_survivor(&pair, &pair.mutants[0], false));
}
