use super::tests::{fixture, request};
use super::*;
use test_client::{Client, corpus};
#[tokio::test]
async fn relevance_quotes_unusable_and_stop_after_first_relevant() {
    for (reply, want) in [
        (corpus("relevant"), Verdict::DoneCandidate),
        (corpus("not_relevant"), Verdict::UndecidedRelevance),
        (corpus("equivalent"), Verdict::JudgeUnavailable),
    ] {
        let f = fixture("test \"$(sh v*)\" = good");
        let mut c = Client::default();
        c.answers.insert("relevant".into(), reply);
        let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
        assert_eq!(r.verdicts.entries[0].verdict, want, "{:?}", r.verdicts);
        if want == Verdict::DoneCandidate {
            assert_eq!(c.completions.lock().unwrap().len(), 1);
        }
        assert!(r.verdicts.entries[0].relevance.iter().all(|m| {
            m.relevance != Relevance::Relevant
                || m.quote
                    .as_deref()
                    .is_some_and(|q| rule::verified_quote(super::tests::TEXT, q))
        }));
    }
    let quote = prompt::object(&corpus("relevant")).unwrap()["quote"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!rule::verified_quote("invented other phrase", &quote));
}
#[tokio::test]
async fn equivalence_marks_cap_unsure_and_one_local_identity() {
    for (answer, want, distinct) in [
        ("equivalent", Verdict::UndecidedRelevance, true),
        ("not_equivalent", Verdict::WeakTest, true),
        ("equivalent", Verdict::UndecidedEquivalence, false),
    ] {
        let f = fixture(
            "observed=$(sh v* 2>/dev/null); if test $? != 0; then printf 'value:1:1: error invalid\\n'; exit 1; fi; test -n \"$observed\"",
        );
        let mut c = Client::default();
        c.answers.insert("not_equivalent".into(), corpus(answer));
        if !distinct {
            c.aliases = [
                ("first".into(), "local".into()),
                ("second".into(), "local".into()),
            ]
            .into();
        }
        let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
        assert_eq!(r.verdicts.entries[0].verdict, want, "{:?}", r.verdicts);
        let mut pair = r.verdicts.entries[0].clone();
        assert!(
            pair.equivalence
                .iter()
                .all(|m| m.marked == (distinct && answer == "equivalent"))
        );
        for mark in &mut pair.equivalence {
            mark.first.answer = EquivalenceAnswer::Unsure;
            mark.marked = false;
        }
        for mutant in pair.mutants.iter().filter(|m| {
            m.kind == MutantKind::Behaviour
                && pair.equivalence.iter().any(|e| e.mutant_id == m.mutant_id)
        }) {
            assert_eq!(rule::real_survivor(&pair, mutant, distinct), distinct);
        }
    }
    equivalence_cap_uses_host_supplied_survivors().await;
}
async fn equivalence_cap_uses_host_supplied_survivors() {
    let f = fixture("test -d .");
    let probe = f.probe(false);
    let mut client = Client::default();
    client
        .answers
        .insert("not_equivalent".into(), corpus("equivalent"));
    let report = prove_requirements(&probe, &client, request(&f, 20)).await;
    client.completions.lock().unwrap().clear();
    let mut pair = report.verdicts.entries[0].clone();
    pair.mutants.clear();
    pair.equivalence.clear();
    pair.survivors.clear();
    let runs: Vec<_> = (0..3)
        .map(|i| {
            let id = format!("host-survivor-{i}");
            pair.mutants.push(AuthoredMutant {
                mutant_id: id.clone(),
                kind: MutantKind::Behaviour,
                breaks: "keeps the good value".into(),
                attempt: 1,
            });
            MutantRun {
                check_id: "check".into(),
                check_digest: String::new(),
                control_key: String::new(),
                mutant_id: id,
                patch_digest: String::new(),
                task_id: pair.task_id.clone(),
                requirement_id: pair.id.clone(),
                kind: MutantKind::Behaviour,
                touched: vec!["value".into()],
                outcome: RunOutcome::Survived { elapsed_ms: 0 },
                counted: false,
                not_counted: None,
                evidence: Evidence {
                    patch: rule::edit_patch(
                        "value",
                        std::str::from_utf8(super::tests::VALUE).unwrap(),
                        &format!(
                            "{}# {i}\n",
                            std::str::from_utf8(super::tests::VALUE).unwrap()
                        ),
                    ),
                    output: None,
                },
            }
        })
        .collect();
    let req = request(&f, 20);
    let circuit = call::Circuit::new(20, &req.call_limits);
    let mut ctx = Context {
        pending: vec![],
        probe: &probe,
        client: &client,
        req,
        verdicts: report.verdicts,
        store: None,
        circuit,
    };
    let view = super::view(&ctx, 0).await.unwrap();
    for _ in 0..3 {
        judges::equivalence(&mut ctx, &mut pair, &view, &runs, None, super::tests::TEXT).await;
    }
    assert_eq!(pair.equivalence.iter().filter(|m| m.marked).count(), 2);
    assert_eq!(pair.equivalence.iter().filter(|m| m.capped).count(), 1);
    assert_eq!(client.completions.lock().unwrap().len(), 4);
    assert_eq!(
        rule::decide(rule::Decision {
            refused: false,
            has_text: true,
            states: &[],
            pair: &pair,
            runs: &runs,
            distinct: true
        }),
        Verdict::WeakTest
    );
    pair.verdict = Verdict::WeakTest;
    let mut pairs = [pair];
    finish::supplements(&mut ctx, &mut pairs, &runs, true);
    assert_eq!(pairs[0].owed.as_ref().unwrap().survivors.len(), 1);
    assert!(pairs[0].owed.as_ref().unwrap().survivors[0].equivalence_unjudged);
}
#[tokio::test]
async fn breaker_truncation_is_unusable_and_never_continued() {
    let f = fixture("test -s v*");
    let c = Client::default();
    for _ in 0..4 {
        c.replies.lock().unwrap().push_back(Ok(
            archon_workflow::llm_client_port::WorkflowAgentOutcome {
                content: corpus("breaker"),
                stop_reason: Some("max_tokens".into()),
                ..Default::default()
            },
        ));
    }
    let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::BreakerUnusable);
    assert_eq!(r.verdicts.entries[0].attempts.len(), 4);
    assert_eq!(r.verdicts.rounds, 0);
    assert!(c.completions.lock().unwrap().is_empty());
}
#[tokio::test]
async fn timeout_opens_circuit_and_judges_continue_only_open_documents() {
    let f = fixture("test \"$(sh v*)\" = good");
    let c = Client {
        timeout: true,
        ..Default::default()
    };
    let mut req = request(&f, 20);
    req.llm_deadline_secs = 1;
    let r = prove_requirements(&f.probe(false), &c, req).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::BreakerUnavailable);
    assert_eq!(c.calls.lock().unwrap().len(), 1);
    assert!(matches!(
        r.verdicts.entries[0].attempts[0].outcome,
        AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            ..
        }
    ));
    let c = Client::default();
    let raw = corpus("relevant");
    let split = raw.find("relevant").unwrap() + 5;
    for (content, stop) in [
        (corpus("breaker"), "end_turn"),
        (corpus("breaker_gut"), "end_turn"),
        (raw[..split].to_string(), "max_tokens"),
        (raw[split..].to_string(), "end_turn"),
    ] {
        c.replies.lock().unwrap().push_back(Ok(
            archon_workflow::llm_client_port::WorkflowAgentOutcome {
                content,
                stop_reason: Some(stop.into()),
                ..Default::default()
            },
        ));
    }
    let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert_eq!(c.calls.lock().unwrap().len(), 2);
    assert_eq!(c.completions.lock().unwrap().len(), 2);
}
