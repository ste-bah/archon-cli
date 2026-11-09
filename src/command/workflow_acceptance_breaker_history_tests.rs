//! Freeze-to-freeze evidence survives changes to the check set.
use super::tests::{fixture, request};
use super::*;
use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use test_client::Client;

#[tokio::test]
async fn new_supplement_runs_carried_survivors_without_authoring() {
    for kills in [true, false] {
        let mut f = fixture("test -d .");
        let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
        assert_eq!(first.verdicts.entries[0].verdict, Verdict::WeakTest);
        assert!(!first.verdicts.entries[0].survivors.is_empty());
        let mut sup = f.contract.acceptance[0].clone();
        sup.id = "SUP-REQ-VALUE-001".into();
        sup.check = serde_json::from_value(serde_json::json!({"kind":"command","command":if kills { "test \"$(sh v*)\" = good" } else { "test -d ." },"cwd":"repo_root"})).unwrap();
        f.contract.supplementary.push(sup);
        f.lock.digest = content_digest(if kills {
            b"new-killing-check"
        } else {
            b"new-surviving-check"
        });
        f.classes.acceptance_digest = f.lock.digest.clone();
        f.lock.baseline_results = BaselineResults::observed(
            f.lock.baseline_commit.clone(),
            &[
                ("check".into(), BaselineOutcome::Passed),
                ("SUP-REQ-VALUE-001".into(), BaselineOutcome::Passed),
            ]
            .into(),
            &f.contract,
        )
        .outcomes;
        f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
        f.classes.entries[0].checks = f
            .lock
            .baseline_results
            .iter()
            .map(|(id, r)| (id.clone(), Some(r.outcome)))
            .collect();
        let client = Client::default();
        let second = prove_requirements(&f.probe(true), &client, request(&f, 50)).await;
        assert!(client.calls.lock().unwrap().is_empty());
        let pair = &second.verdicts.entries[0];
        if kills {
            assert!(pair.survivors.is_empty());
            assert!(
                pair.killed_by
                    .values()
                    .all(|checks| checks.contains(&"SUP-REQ-VALUE-001".into()))
            );
        } else {
            assert_eq!(pair.verdict, Verdict::WeakTest);
            assert!(!pair.survivors.is_empty());
        }
    }
}

#[tokio::test]
async fn restore_keeps_evidence_after_changed_judge_identity_or_logic() {
    for changed_logic in [false, true] {
        let f = fixture("test -d .");
        let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
        assert!(!first.verdicts.entries[0].survivors.is_empty());
        let mut client = Client::default();
        if changed_logic {
            let path = first
                .store
                .as_ref()
                .unwrap()
                .parent()
                .unwrap()
                .join("survivor-history.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value["logic"] = (BREAKER_LOGIC + 1).into();
            std::fs::write(path, value.to_string()).unwrap();
        } else {
            client
                .aliases
                .insert("first".into(), "changed-identity".into());
        }
        let probe = f.probe(true);
        let req = request(&f, 50);
        let circuit = call::Circuit::new(20, &req.call_limits);
        let mut verdicts = first.verdicts.clone();
        verdicts.judges.equivalence[0] = identity(&client, "breaker_equivalence", FIRST_MODEL);
        let ctx = Context {
            pending: vec![],
            probe: &probe,
            client: &client,
            req,
            verdicts,
            store: first.store.clone(),
            circuit,
        };
        assert!(history::load(&ctx).is_some());
        let second = prove_requirements(&probe, &client, request(&f, 50)).await;
        assert!(client.calls.lock().unwrap().is_empty());
        assert_eq!(second.verdicts.entries[0].verdict, Verdict::WeakTest);
        if !changed_logic {
            assert!(!client.completions.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn unavailable_body_removal_is_listed_with_literal_parse_feedback() {
    let f = fixture("test \"$(sh v*)\" = good");
    let mut client = Client::default();
    client
        .answers
        .insert("breaker_gut".into(), test_client::corpus("not_relevant"));
    let report = prove_requirements(&f.probe(false), &client, request(&f, 50)).await;
    let pair = &report.verdicts.entries[0];
    assert_eq!(pair.verdict, Verdict::BodyGutUnavailable);
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(
        calls[2]
            .task
            .contains("Previous reply failed: proposal_invalid: missing field `key`")
    );
    assert!(pair.attempts.iter().any(|a|matches!(&a.outcome, AttemptOutcome::Unusable { reason } if reason.contains("missing field `key`"))));
}

#[tokio::test]
async fn unusable_behaviour_reply_with_a_killed_gut_is_not_unmutable() {
    let f = fixture("test \"$(sh v*)\" = good");
    let mut client = Client::default();
    client.answers.insert(
        "breaker".into(),
        test_client::corpus("breaker_r18_attempt1"),
    );
    let report = prove_requirements(&f.probe(false), &client, request(&f, 50)).await;
    let pair = &report.verdicts.entries[0];
    assert_eq!(pair.verdict, Verdict::BreakerUnusable);
    assert!(pair.survivors.is_empty());
    assert!(!pair.killed_by.is_empty());
    assert!(pair.mutants.iter().all(|m| m.kind == MutantKind::BodyGut));
    assert_eq!(client.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn later_freeze_finishes_a_gut_call_stopped_by_the_previous_cap() {
    let f = fixture("test \"$(sh v*)\" = good");
    let probe = f.probe(true);
    let mut req = request(&f, 50);
    req.call_limits.per_pair = archon_core::config::AuditLimit::Finite(1);
    let first = prove_requirements(&probe, &Client::default(), req).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::BudgetExhausted);
    let client = Client::default();
    let second = prove_requirements(&probe, &client, request(&f, 50)).await;
    assert_eq!(second.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].task.contains("Pick one function or block"));
}

#[tokio::test]
async fn unchanged_deliverable_at_new_base_reruns_evidence_without_authoring() {
    let mut f = fixture("test -d .");
    let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::WeakTest);
    std::fs::write(f.source.path().join("unrelated"), "another base").unwrap();
    let snapshot = super::view_tests::snapshot(&f, &["value"]);
    f.lock.baseline_commit = Some(snapshot.commit.clone());
    f.classes.baseline_commit = Some(snapshot.commit.clone());
    f.classes.base_snapshot = Some(snapshot);
    let client = Client::default();
    let second = prove_requirements(&f.probe(true), &client, request(&f, 50)).await;
    assert!(client.calls.lock().unwrap().is_empty());
    assert_ne!(second.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert!(!second.verdicts.entries[0].survivors.is_empty());
    assert!(second.verdicts.runs_executed > 0);
}
#[tokio::test]
async fn missing_survivor_patch_is_logged_and_dropped() {
    let f = fixture("test -d .");
    let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    let survivor = &first.verdicts.entries[0].survivors[0];
    std::fs::remove_file(first.store.as_ref().unwrap().join(&survivor.patch)).unwrap();
    let second = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    let pair = &second.verdicts.entries[0];
    assert_ne!(pair.verdict, Verdict::DoneCandidate);
    assert!(
        !pair
            .survivors
            .iter()
            .any(|s| s.mutant_id == survivor.mutant_id)
    );
    assert_ne!(pair.verdict, Verdict::HostUnrun);
    assert!(
        pair.attempts
            .iter()
            .flat_map(|a| &a.refusals)
            .any(|r| r.starts_with("survivor_dropped:"))
    );
}
#[tokio::test]
async fn gut_build_failure_retries_once_with_literal_location_then_is_listed() {
    let f = fixture(
        "if grep -q ':;' value; then printf 'value:1:1: error: invalid body\\n' >&2; exit 1; fi; test \"$(sh v*)\" = good",
    );
    let client = Client::default();
    let report = prove_requirements(&f.probe(false), &client, request(&f, 50)).await;
    let guts: Vec<_> = client
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.task.contains("Pick one function or block"))
        .map(|c| c.task.clone())
        .collect();
    assert_eq!(guts.len(), 2);
    assert!(guts[1].contains("Previous reply failed: value:1"));
    assert_eq!(
        report.verdicts.entries[0].verdict,
        Verdict::BodyGutUnavailable
    );
}
#[tokio::test]
async fn two_file_view_authors_one_gut_only_for_the_declared_file() {
    let f = fixture("unused");
    let probe = f.probe(false);
    let client = Client::default();
    let req = request(&f, 50);
    let first = prove_requirements(&probe, &client, request(&f, 50)).await;
    let mut pair = first.verdicts.entries[0].clone();
    pair.attempts.clear();
    let mut view = materialise_view(
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
    view.files.insert(
        "other".into(),
        super::super::super::breaker_view::ViewFile {
            object: String::new(),
            bytes: super::tests::VALUE.to_vec(),
        },
    );
    let circuit = call::Circuit::new(20, &req.call_limits);
    let client = Client::default();
    let mut ctx = Context {
        pending: vec![],
        probe: &probe,
        client: &client,
        req,
        verdicts: first.verdicts,
        store: None,
        circuit,
    };
    let supplied = gut::author(&mut ctx, &mut pair, &view, super::tests::TEXT, "", 1).await;
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert_eq!(supplied.len(), 1);
    assert!(supplied[0].patch.contains("--- a/value"));
    let raw = test_client::corpus("breaker_gut").replace("\"value\"", "\"other\"");
    assert_eq!(
        prompt::parse_gut(&raw, "task", "id", super::tests::TEXT, &view, "[\"value\"]")
            .err()
            .unwrap(),
        "key_not_target"
    );
}
