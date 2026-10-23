//! Per-verdict acceptance (#434d) through the whole audit, over the real
//! store: a reply's verdicts that pass every check are saved at once; only
//! the obligations still without a verdict are asked again; a document
//! refused as a whole gives nothing; the set without a verdict shrinking is
//! progress; and an operational end names only what is left, while a retry
//! reuses every verdict already saved.

use super::reask::wide_corpus;
use super::*;

/// Words that are nowhere in TASK-WS-001: a paraphrase, not a quote.
const PARAPHRASE: &str = "tests may run somewhere other than the shared store";

/// A verdict for every id of `ids`: true and verbatim, except a false one
/// with a paraphrased quote for each id in `bad`.
fn batch_reply(ids: &[String], bad: &[String]) -> String {
    let verdicts: Vec<_> = ids
        .iter()
        .map(|id| {
            if bad.contains(id) {
                serde_json::json!({"obligation_id": id, "necessarily_true": false, "weakest_task_id": "TASK-WS-001", "reason": "the store may stay empty", "quoted_task_text": PARAPHRASE, "supporting_task_id": "", "supporting_text": ""})
            } else {
                serde_json::json!({"obligation_id": id, "necessarily_true": true, "weakest_task_id": "", "reason": "obliged", "quoted_task_text": "", "supporting_task_id": "TASK-WS-001", "supporting_text": SUPPORT})
            }
        })
        .collect();
    serde_json::json!({ "verdicts": verdicts }).to_string()
}

/// The re-ask turn of call `n` (from 0).
fn reask_turn(critic: &FakeCritic, n: usize) -> String {
    let conversations = critic.conversations.lock().unwrap();
    let conversation = &conversations[n];
    assert_eq!(conversation.len(), 3, "{conversation:#?}");
    assert_eq!(
        conversation[0], conversations[0][0],
        "the question is unchanged"
    );
    conversation[2]["content"].as_str().unwrap().to_string()
}

fn still_asked(turn: &str) -> String {
    let marker = "no others: ";
    let rest = &turn[turn.find(marker).expect("the ids still asked") + marker.len()..];
    rest[..rest.find(';').expect("end of ids")].to_string()
}

#[tokio::test]
async fn fidelity_seven_verdicts_are_kept_and_only_the_eighth_is_asked_again() {
    let (temp, ids) = wide_corpus();
    assert_eq!(ids.len(), 8, "one call batch of eight");
    let bad = vec![ids[7].clone()];
    let critic = FakeCritic::new(vec![
        Ok(batch_reply(&ids, &bad)),
        Ok(batch_reply(&bad, &[])),
    ]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 2, "one call, then exactly one re-ask");
    let turn = reask_turn(&critic, 1);
    assert!(
        turn.contains(&format!(
            "- {}: false verdict for {} quotes text in `quoted_task_text` that does not appear verbatim in task 'TASK-WS-001'",
            ids[7], ids[7]
        )),
        "the one error, exactly: {turn}"
    );
    assert_eq!(still_asked(&turn), ids[7]);
    for kept in &ids[..7] {
        assert!(
            !turn.contains(kept.as_str()),
            "{kept} is not asked again: {turn}"
        );
    }
    assert_eq!(
        record_files(temp.path()).len(),
        8,
        "one record per obligation"
    );
    assert!(
        evaluation.report.contains("8 asked of"),
        "{}",
        evaluation.report
    );
}

/// A refused document gives no verdict, however many of its verdicts pass.
#[tokio::test]
async fn fidelity_a_refused_document_keeps_no_verdict() {
    let good: Vec<serde_json::Value> = serde_json::from_str::<serde_json::Value>(&reply(&[]))
        .unwrap()["verdicts"]
        .as_array()
        .unwrap()
        .clone();
    let mut extra = good[1].clone();
    extra["obligation_id"] = "OTHER-1".into();
    let mut stray = good.clone();
    stray[1]["zz_stray"] = "".into();
    for verdicts in [
        vec![good[0].clone(), good[1].clone(), good[1].clone()],
        vec![good[0].clone(), good[1].clone(), extra],
        stray,
    ] {
        let temp = corpus();
        let document = serde_json::json!({ "verdicts": verdicts }).to_string();
        let attempts = 1 + FIDELITY_NO_PROGRESS_WINDOW;
        let critic = FakeCritic::new(vec![Ok(document.clone()); attempts]);
        let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
        assert_eq!(critic.calls(), attempts, "{document}");
        let error = evaluation.operational_error().expect("operational");
        assert!(
            error.contains(r#"["AC-WS-001", "G-WS-001"]"#),
            "both are still without a verdict: {error}"
        );
        assert!(
            record_files(temp.path()).is_empty(),
            "nothing saved: {document}"
        );
    }
}

/// Every refusal after the first reply is of a class already seen, yet each
/// reply leaves one obligation fewer without a verdict: progress, for longer
/// than the no-progress window.
#[tokio::test]
async fn fidelity_the_set_without_a_verdict_shrinking_is_progress() {
    let (temp, ids) = wide_corpus();
    let mut replies: Vec<_> = (0..ids.len())
        .map(|kept| Ok(batch_reply(&ids, &ids[kept..])))
        .collect();
    replies.push(Ok(batch_reply(&ids, &[])));
    assert!(ids.len() - 1 > FIDELITY_NO_PROGRESS_WINDOW);
    let critic = FakeCritic::new(replies);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), ids.len() + 1);
    for n in 1..=ids.len() {
        assert_eq!(
            still_asked(&reask_turn(&critic, n)),
            ids[n - 1..].join(", ")
        );
    }
    assert_eq!(record_files(temp.path()).len(), 8);
}

/// The live failure: one verdict quotes words its task does not contain,
/// again and again. Its seven partners are saved at once; the operational
/// error names it alone; the retry asks it alone.
#[tokio::test]
async fn fidelity_a_verdict_that_never_quotes_verbatim_ends_alone_and_the_rest_are_reused() {
    let (temp, ids) = wide_corpus();
    let bad = vec![ids[7].clone()];
    let attempts = 1 + FIDELITY_NO_PROGRESS_WINDOW;
    let critic = FakeCritic::new(vec![Ok(batch_reply(&ids, &bad)); attempts]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), attempts);
    let error = evaluation.operational_error().expect("operational");
    assert!(
        error.contains(&format!(
            "no usable verdict for [\"{}\"] after {attempts} attempts",
            ids[7]
        )),
        "only the obligation still without a verdict: {error}"
    );
    assert!(error.contains("does not appear verbatim"), "{error}");
    assert_eq!(
        record_files(temp.path()).len(),
        7,
        "the seven passed verdicts are saved"
    );

    let retry = FakeCritic::new(vec![Ok(batch_reply(&bad, &[]))]);
    let evaluation = evaluate(temp.path(), Ok(retry.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(retry.calls(), 1);
    let prompt = retry.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains(&format!("\"id\":\"{}\"", ids[7])),
        "{prompt}"
    );
    for kept in &ids[..7] {
        assert!(
            !prompt.contains(&format!("\"id\":\"{kept}\"")),
            "{kept} is reused, never asked again"
        );
    }
    assert!(
        evaluation.report.contains("1 asked of") && evaluation.report.contains("7 served from"),
        "{}",
        evaluation.report
    );
}

/// A re-ask reply that answers a kept obligation again with the other
/// meaning is logged and nothing more: the kept verdict stands, no
/// contradiction is reported and its record holds one entry.
#[tracing_test::traced_test]
#[tokio::test]
async fn fidelity_a_kept_verdict_answered_again_with_the_other_meaning_is_only_logged() {
    let temp = corpus();
    let ids = ["AC-WS-001".to_string(), "G-WS-001".to_string()];
    let critic = FakeCritic::new(vec![
        Ok(batch_reply(&ids, &ids[1..])),
        Ok(reply(&["AC-WS-001"])),
    ]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 2);
    assert_eq!(still_asked(&reask_turn(&critic, 1)), "G-WS-001");
    assert!(logs_contain(
        "answered a kept verdict again with the other meaning"
    ));
    assert!(logs_contain("obligation_id=AC-WS-001"));
    assert!(logs_contain("kept_necessarily_true=true"));
    assert!(logs_contain("reanswered_necessarily_true=false"));
    assert!(
        evaluation
            .findings
            .iter()
            .all(|finding| !finding.text.contains("contradict")),
        "no contradiction is recorded"
    );
    assert!(
        !evaluation
            .findings
            .iter()
            .any(|finding| finding.text.starts_with("obligation AC-WS-001")),
        "the kept true verdict stands"
    );
    let record = record_files(temp.path())
        .into_iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .find(|text| text.contains("\"id\": \"AC-WS-001\""))
        .expect("AC-WS-001's record");
    let record: serde_json::Value = serde_json::from_str(&record).unwrap();
    let entries = record["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "no second entry");
    assert_eq!(entries[0]["verdict"]["necessarily_true"], true);
}

/// D5: replies that never give a usable verdict are the critic's own
/// unusable replies, carried as an authored judge reply so the author loop
/// counts them without charging the host's operational window; a provider
/// that gives no reply at all stays the host's operational error.
#[tokio::test]
async fn fidelity_an_unusable_critic_reply_is_authored_and_a_provider_fault_is_operational() {
    let (temp, ids) = wide_corpus();
    let bad = vec![ids[7].clone()];
    let critic = FakeCritic::new(vec![
        Ok(batch_reply(&ids, &bad));
        1 + FIDELITY_NO_PROGRESS_WINDOW
    ]);
    let envelope = evaluate(temp.path(), Ok(critic))
        .await
        .into_envelope()
        .unwrap();
    let error = envelope.operational_error.expect("no usable verdict");
    assert_eq!(error.kind, "authored_judge_reply", "{}", error.text);

    let (temp, _) = wide_corpus();
    let provider = FakeCritic::new(vec![Err("upstream unavailable".into())]);
    let envelope = evaluate(temp.path(), Ok(provider))
        .await
        .into_envelope()
        .unwrap();
    let error = envelope.operational_error.expect("no reply at all");
    assert_eq!(error.kind, "gate_operational", "{}", error.text);
}
