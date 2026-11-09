use super::tests::{TEXT, fixture, request};
use super::*;
use test_client::{Client, corpus};
fn local() -> Client {
    Client {
        aliases: [
            ("first".into(), "local".into()),
            ("second".into(), "local".into()),
        ]
        .into(),
        ..Default::default()
    }
}
#[tokio::test]
async fn single_model_differs_owes_supplement_and_asks_once_per_survivor() {
    let f = fixture("test -s v*");
    let c = local();
    let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
    let pair = &r.verdicts.entries[0];
    assert_eq!(pair.verdict, Verdict::WeakTest);
    assert_eq!(c.completions.lock().unwrap().len(), 2);
    let owed = pair.owed.as_ref().unwrap();
    assert_eq!(owed.survivors.len(), 3);
    assert!(owed.survivors.iter().all(|s| !s.equivalence_unjudged));
}
#[tokio::test]
async fn undecided_survivors_survive_next_freeze_without_a_new_breaker_call() {
    let f = fixture("test -n \"$(sh v*)\"");
    let p = f.probe(true);
    let mut c = local();
    c.answers
        .insert("not_equivalent".into(), corpus("equivalent"));
    let first = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(
        first.verdicts.entries[0].verdict,
        Verdict::UndecidedEquivalence
    );
    let mut c = local();
    c.answers
        .insert("not_equivalent".into(), corpus("equivalent"));
    let second = prove_requirements(&p, &c, request(&f, 20)).await;
    assert!(c.calls.lock().unwrap().is_empty());
    assert_eq!(
        second.verdicts.entries[0].verdict,
        Verdict::UndecidedEquivalence
    );
    assert_eq!(
        second.verdicts.entries[0].survivors.len(),
        first.verdicts.entries[0].survivors.len()
    );
}
#[tokio::test]
async fn judge_outage_is_reasked_at_next_freeze() {
    let f = fixture("test -n \"$(sh v*)\"");
    let p = f.probe(true);
    let c = Client {
        judge_fail: true,
        ..local()
    };
    let first = prove_requirements(&p, &c, request(&f, 20)).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::JudgeUnavailable);
    let c = local();
    let second = prove_requirements(&p, &c, request(&f, 20)).await;
    assert!(c.calls.lock().unwrap().is_empty());
    assert!(!c.completions.lock().unwrap().is_empty());
    assert_eq!(second.verdicts.entries[0].verdict, Verdict::WeakTest);
}
#[tokio::test]
async fn relevance_marks_are_written_once_across_rounds() {
    let f = fixture("test \"$(sh v*)\" = good");
    let mut c = Client::default();
    c.answers.insert("relevant".into(), corpus("not_relevant"));
    let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.rounds, 2);
    assert_eq!(c.completions.lock().unwrap().len(), 2);
    assert_eq!(r.verdicts.entries[0].relevance.len(), 2);
}
#[tokio::test]
async fn malformed_then_valid_reply_gets_one_content_based_retry() {
    let f = fixture("test -d .");
    let c = Client::default();
    c.replies.lock().unwrap().extend(
        [
            corpus("breaker_r18_attempt1"),
            corpus("breaker_gut"),
            corpus("breaker"),
        ]
        .map(|content| {
            Ok(archon_workflow::llm_client_port::WorkflowAgentOutcome {
                content,
                stop_reason: Some("end_turn".into()),
                ..Default::default()
            })
        }),
    );
    let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::WeakTest);
    let calls = c.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[2].task.contains("breaks_not_in_requirement"));
    assert!(calls[2].task.contains("data_store"));
}
#[test]
fn parser_accepts_extra_keys_and_names_missing_required_fields() {
    let f = fixture("unused");
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    let mut raw = super::parse_tests::proposals();
    raw["description"] = "extra".into();
    raw["mutants"][0]["description"] = "extra".into();
    raw["mutants"][0]["edits"][0]["description"] = "extra".into();
    assert_eq!(
        prompt::parse(&raw.to_string(), "task", "item", TEXT, &view)
            .unwrap()
            .mutants
            .len(),
        3
    );
    raw["mutants"][0]["edits"][0]
        .as_object_mut()
        .unwrap()
        .remove("find");
    let parsed = prompt::parse(&raw.to_string(), "task", "item", TEXT, &view).unwrap();
    assert_eq!(parsed.mutants.len(), 2);
    assert!(parsed.refusals[0].contains("proposal_invalid: missing field `find`"));
}

#[test]
fn per_pair_and_global_caps_are_deterministic_and_prioritise_unprocessed() {
    let limits = archon_core::config::BreakerCallLimits {
        per_pair: archon_core::config::AuditLimit::Finite(1),
        global: archon_core::config::AuditLimit::Finite(2),
        ..Default::default()
    };
    let mut c = call::Circuit::new(20, &limits);
    assert!(c.remaining().is_ok());
    assert_eq!(c.remaining().unwrap_err().0, UnavailableCause::Budget);
    c.pair = 1;
    assert!(c.remaining().is_ok());
    c.pair = 2;
    assert_eq!(c.remaining().unwrap_err().0, UnavailableCause::Budget);
    assert!(!c.failed);
    let mut pairs: Vec<PairVerdict> = (0..3).map(|i| serde_json::from_value(serde_json::json!({"task_id":i.to_string(),"id":"item","verdict":"host_unrun","killed_by":{}})).unwrap()).collect();
    let mut previous = ProofVerdicts {
        schema: VERDICT_SCHEMA,
        base_commit: String::new(),
        acceptance_digest: String::new(),
        baseline_results_digest: String::new(),
        snapshot_digest: String::new(),
        call_limits: None,
        rounds: 1,
        runs_executed: 0,
        runs_reused: 0,
        judges: JudgeIdentities {
            breaker: ModelIdentity {
                provider_id: None,
                model: "one".into(),
            },
            relevance: ModelIdentity {
                provider_id: None,
                model: "one".into(),
            },
            equivalence: vec![],
            equivalence_distinct: false,
        },
        refusals: vec![],
        entries: {
            pairs[0].verdict = Verdict::UndecidedRelevance;
            pairs[1].verdict = Verdict::DoneCandidate;
            pairs[..2].to_vec()
        },
    };
    assert_eq!(
        history::order(&pairs, &Some(previous.clone())),
        vec![2, 0, 1]
    );
    previous.entries[0].run_budget_exhausted = true;
    assert_eq!(
        history::order(&pairs, &Some(previous.clone())),
        vec![0, 2, 1]
    );
    previous.entries[0].verdict = Verdict::BudgetExhausted;
    previous.entries[0].attempts.push(AttemptRecord {
        attempt: 1,
        reply_key: String::new(),
        outcome: AttemptOutcome::Accepted { mutants: vec![] },
        refusals: vec![],
    });
    previous.entries[1].verdict = Verdict::BudgetExhausted;
    previous.entries[1].attempts.push(AttemptRecord {
        attempt: 1,
        reply_key: String::new(),
        outcome: AttemptOutcome::Unavailable {
            cause: UnavailableCause::Budget,
            reason: "call_cap_exhausted".into(),
        },
        refusals: vec![],
    });
    assert_eq!(history::order(&pairs, &Some(previous)), vec![0, 1, 2]);
}
#[tokio::test]
async fn cap_exhaustion_is_a_budget_verdict() {
    for (per_pair, global) in [(1, 10), (10, 1)] {
        let f = fixture("test \"$(sh v*)\" = good");
        let c = Client::default();
        let mut req = request(&f, 20);
        req.call_limits = archon_core::config::BreakerCallLimits {
            per_pair: archon_core::config::AuditLimit::Finite(per_pair),
            global: archon_core::config::AuditLimit::Finite(global),
            ..Default::default()
        };
        let r = prove_requirements(&f.probe(false), &c, req).await;
        assert_eq!(r.verdicts.entries[0].verdict, Verdict::BudgetExhausted);
    }
}

#[test]
fn embedded_call_limits_and_project_overrides_follow_config_section() {
    let config: archon_core::config::ArchonConfig =
        toml::from_str(include_str!("../../config.toml")).unwrap();
    assert_eq!(config.workflow.breaker_calls.per_pair.finite(), Some(32));
    assert_eq!(config.workflow.breaker_calls.global.finite(), Some(256));
    let config: archon_core::config::WorkflowRuntimeConfig =
        toml::from_str("[breaker_calls]\nper_pair = 3\nglobal = 7\n").unwrap();
    assert_eq!(config.breaker_calls.per_pair.finite(), Some(3));
    assert_eq!(config.breaker_calls.global.finite(), Some(7));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    std::fs::write(
        &path,
        "[workflow.breaker_calls]\nper_pair = 3\nglobal = 7\n",
    )
    .unwrap();
    let loaded = archon_core::config::load_config_from(path.clone()).unwrap();
    assert_eq!(
        loaded.workflow.breaker_calls.sources["per_pair"]["path"],
        path.display().to_string()
    );
    assert_eq!(
        loaded.workflow.breaker_calls.sources["global"]["layer"],
        "file"
    );
}

#[test]
fn unrelated_requirement_quote_does_not_validate_the_mutants_phrase() {
    let value = prompt::object(&corpus("relevant")).unwrap();
    let quote = value["quote"].as_str().unwrap();
    assert!(rule::verified_quote(TEXT, quote));
    assert!(!rule::verified_quote("for every input", quote));
    assert!(!rule::verified_quote(quote, "for every input"));
}

#[tokio::test]
async fn existence_check_retains_a_nested_declared_deliverable() {
    let mut f = fixture("test -f crates/archon-trading/src/data_store.rs");
    let input: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/breaker_reply_corpus/breaker_r18.input.json"
    ))
    .unwrap();
    for (key, value) in input["files"].as_object().unwrap() {
        let path = f.source.path().join(key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.as_str().unwrap()).unwrap();
    }
    let snapshot = super::view_tests::snapshot(&f, &["crates"]);
    f.lock.baseline_commit = Some(snapshot.commit.clone());
    f.classes.baseline_commit = Some(snapshot.commit.clone());
    f.classes.base_snapshot = Some(snapshot);
    f.classes.entries[0].deliverables[0].artifact_path = "crates".into();
    let text = format!("- REQ-VALUE-001: {}\n", input["text"].as_str().unwrap());
    let mut c = Client::default();
    c.answers
        .insert("breaker".into(), corpus("breaker_r18_attempt1"));
    c.answers
        .insert("breaker_gut".into(), corpus("breaker_r18_gut"));
    let mut req = request(&f, 50);
    req.prd_text = &text;
    let r = prove_requirements(&f.probe(false), &c, req).await;
    assert_eq!(r.verdicts.entries[0].verdict, Verdict::WeakTest);
    assert!(r.verdicts.entries[0].owed.is_some());
    // Both recorded replies are accepted without a behaviour retry.
    let pair = &r.verdicts.entries[0];
    assert!(pair.mutants.iter().any(|m| m.kind == MutantKind::Behaviour));
    assert!(pair.mutants.iter().any(|m| m.kind == MutantKind::BodyGut));
    assert!(
        pair.attempts
            .iter()
            .all(|a| matches!(a.outcome, AttemptOutcome::Accepted { .. }))
    );
    let calls = c.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| {
        call.task
            .contains("crates/archon-trading/src/data_store.rs")
    }));
}

#[tokio::test]
async fn answered_single_judge_marks_are_judged_and_all_survivors_are_owed() {
    for (answer, owed) in [("equivalent", 3), ("not_equivalent", 3)] {
        let f = fixture("test -d .");
        let mut c = local();
        c.answers.insert("not_equivalent".into(), corpus(answer));
        let r = prove_requirements(&f.probe(false), &c, request(&f, 20)).await;
        let pair = &r.verdicts.entries[0];
        assert_eq!(pair.verdict, Verdict::WeakTest);
        assert!(pair.survivors.iter().all(|s| !s.equivalence_unjudged));
        assert_eq!(pair.owed.as_ref().unwrap().survivors.len(), owed);
        assert_eq!(pair.owed.as_ref().unwrap().survivors, pair.survivors);
        assert_eq!(c.completions.lock().unwrap().len(), 2);
    }
}

#[test]
fn zero_call_limits_are_rejected_and_unlimited_is_explicit() {
    for field in ["per_pair", "global"] {
        let text = format!("[breaker_calls]\n{field} = 0\n");
        assert!(toml::from_str::<archon_core::config::WorkflowRuntimeConfig>(&text).is_err());
        let text = format!("[breaker_calls]\n{field} = \"unlimited\"\n");
        assert!(toml::from_str::<archon_core::config::WorkflowRuntimeConfig>(&text).is_ok());
    }
}
