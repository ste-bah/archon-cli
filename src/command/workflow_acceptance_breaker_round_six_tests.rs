use super::tests::{fixture, request};
use super::*;
use test_client::Client;

#[tokio::test]
async fn both_behaviour_attempts_unviable_with_killed_gut_are_listed() {
    let f = fixture(
        "if ! grep -q 'printf good;' value && ! grep -q ':;' value; then printf 'value:1:1: error: invalid behaviour\\n' >&2; exit 1; fi; test \"$(sh v*)\" = good",
    );
    let client = Client::default();
    let report = prove_requirements(&f.probe(false), &client, request(&f, 50)).await;
    let pair = &report.verdicts.entries[0];
    assert_eq!(pair.verdict, Verdict::NoViableBehaviourMutant);
    assert!(
        pair.attempts
            .iter()
            .any(|a| a.attempt == 2 && !a.reply_key.starts_with("body_gut:"))
    );
    assert!(
        pair.mutants
            .iter()
            .any(|m| m.kind == MutantKind::BodyGut && pair.killed_by.contains_key(&m.mutant_id))
    );
}

#[tokio::test]
async fn carried_and_current_survivors_both_reach_the_supplement() {
    let f = fixture("test -d .");
    let probe = f.probe(true);
    let client = Client::default();
    let first = prove_requirements(&probe, &client, request(&f, 50)).await;
    let mut pair = first.verdicts.entries[0].clone();
    let carried = pair.survivors[0].clone();
    let current = pair.survivors[1].clone();
    pair.survivors = vec![carried.clone()];
    let run: MutantRun = serde_json::from_value(serde_json::json!({
        "check_id":"check","check_digest":"","control_key":"","mutant_id":current.mutant_id,
        "patch_digest":"","task_id":pair.task_id,"requirement_id":pair.id,"kind":current.kind,
        "touched":["value"],"outcome":{"outcome":"survived","elapsed_ms":1},"counted":false,
        "evidence":{"patch":current.patch,"output":current.output}
    }))
    .unwrap();
    let req = request(&f, 50);
    let circuit = call::Circuit::new(20, &req.call_limits);
    let mut ctx = Context {
        pending: vec![],
        probe: &probe,
        client: &client,
        req,
        verdicts: first.verdicts,
        store: first.store,
        circuit,
    };
    // Carried evidence has no judge mark in this freeze.
    pair.equivalence
        .retain(|m| m.mutant_id != carried.mutant_id);
    let mut pairs = vec![pair];
    finish::supplements(&mut ctx, &mut pairs, &[run], true);
    let expected: BTreeSet<_> = [carried.mutant_id, current.mutant_id].into();
    assert_eq!(
        pairs[0]
            .survivors
            .iter()
            .map(|s| s.mutant_id.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        pairs[0]
            .owed
            .as_ref()
            .unwrap()
            .survivors
            .iter()
            .map(|s| s.mutant_id.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );
}

#[tokio::test]
async fn missing_patch_is_dropped_and_other_pair_history_saves_across_three_freezes() {
    let mut f = fixture("test -d .");
    let mut other = f.classes.entries[0].clone();
    other.task_id = "other-task".into();
    f.classes.entries.push(other);
    let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 100)).await;
    let stale = first.verdicts.entries[0].survivors[0].mutant_id.clone();
    let patch = &first.verdicts.entries[0].survivors[0].patch;
    std::fs::remove_file(first.store.as_ref().unwrap().join(patch)).unwrap();
    let second = prove_requirements(&f.probe(true), &Client::default(), request(&f, 100)).await;
    let pair = &second.verdicts.entries[0];
    assert_ne!(pair.verdict, Verdict::HostUnrun);
    assert!(
        pair.attempts
            .iter()
            .flat_map(|a| &a.refusals)
            .any(|r| r.contains("patch_file_missing"))
    );
    assert!(!pair.survivors.iter().any(|s| s.mutant_id == stale));
    let path = second
        .store
        .as_ref()
        .unwrap()
        .parent()
        .unwrap()
        .join("survivor-history.json");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let evidence: ProofVerdicts = serde_json::from_value(saved["evidence"].clone()).unwrap();
    assert_eq!(evidence, second.verdicts);
    assert!(!evidence.entries[1].survivors.is_empty());
    let third = prove_requirements(&f.probe(true), &Client::default(), request(&f, 100)).await;
    assert!(
        third
            .verdicts
            .entries
            .iter()
            .all(|p| p.verdict != Verdict::HostUnrun)
    );
    assert!(
        !third.verdicts.entries[0]
            .survivors
            .iter()
            .any(|s| s.mutant_id == stale)
    );
}

#[tokio::test]
async fn inapplicable_or_unviable_carried_patch_is_dropped_with_literal_reason() {
    for (before, outcome) in [
        ("no longer present\n", "Inapplicable"),
        (
            std::str::from_utf8(super::tests::VALUE).unwrap(),
            "Unviable",
        ),
    ] {
        let f = fixture(
            "if grep -q changed value; then printf 'value:1:1: error: no longer builds\\n' >&2; exit 1; fi; test -d .",
        );
        let first = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
        let mut previous = first.verdicts.clone();
        let old = &mut previous.entries[0];
        let mutant = SuppliedMutant {
            task_id: old.task_id.clone(),
            requirement_id: old.id.clone(),
            kind: MutantKind::Behaviour,
            patch: rule::edit_patch("value", before, "changed\n"),
        };
        let id = mutant.mutant_id();
        let patch = old.survivors[0].patch.clone();
        std::fs::write(first.store.as_ref().unwrap().join(&patch), &mutant.patch).unwrap();
        old.mutants[0].mutant_id = id.clone();
        old.patch_keys = [(id.clone(), patch.clone())].into();
        old.mutants.truncate(1);
        old.survivors.truncate(1);
        old.survivors[0].mutant_id = id.clone();
        let path = first
            .store
            .as_ref()
            .unwrap()
            .parent()
            .unwrap()
            .join("survivor-history.json");
        let mut history: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        history["evidence"] = serde_json::to_value(previous).unwrap();
        std::fs::write(path, history.to_string()).unwrap();
        let second = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
        let pair = &second.verdicts.entries[0];
        assert_ne!(pair.verdict, Verdict::HostUnrun);
        assert!(!pair.survivors.iter().any(|s| s.mutant_id == id));
        assert!(!pair.mutants.iter().any(|m| m.mutant_id == id));
        assert!(
            pair.attempts
                .iter()
                .flat_map(|a| &a.refusals)
                .any(|r| r.contains("survivor_unviable:") && r.contains(outcome))
        );
    }
}

#[tokio::test]
async fn parsed_second_gut_still_gets_one_engine_retry() {
    let f = fixture(
        "if grep -q ':;' value; then printf 'value:1:1: error: invalid body\\n' >&2; exit 1; fi; test \"$(sh v*)\" = good",
    );
    let client = Client::default();
    for name in [
        "breaker",
        "breaker_r18_round5_attempt1_gut1",
        "breaker_gut",
        "relevant",
        "breaker_gut",
    ] {
        client.replies.lock().unwrap().push_back(Ok(
            archon_workflow::llm_client_port::WorkflowAgentOutcome {
                content: test_client::corpus(name),
                stop_reason: Some("end_turn".into()),
                ..Default::default()
            },
        ));
    }
    let report = prove_requirements(&f.probe(false), &client, request(&f, 50)).await;
    assert_eq!(
        report.verdicts.entries[0].verdict,
        Verdict::BodyGutUnavailable
    );
    let calls = client.calls.lock().unwrap();
    let guts: Vec<_> = calls
        .iter()
        .filter(|c| c.task.contains("Pick one function or block"))
        .collect();
    assert_eq!(guts.len(), 3);
    assert!(guts[2].task.contains("Previous reply failed: value:1"));
}
