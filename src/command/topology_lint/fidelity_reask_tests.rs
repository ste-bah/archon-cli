//! The re-ask: the original question, then only the last rejected reply and
//! its exact parse error with the allowed keys. The critic is asked again
//! while its replies make progress — each refused reply's first error falls
//! in a (verdict, kind) class no earlier reply had. The same class again is
//! no progress, whatever its error text; the audit is operational once
//! [`FIDELITY_NO_PROGRESS_WINDOW`] of them come in a row. A verdict with a
//! missing or unknown field is never accepted.

use super::*;

const VERDICT_KEYS: [&str; 5] = [
    "obligation_id",
    "necessarily_true",
    "weakest_task_id",
    "reason",
    "quoted_task_text",
];

/// A complete true-verdict document whose verdict `index` carries one
/// stray key `name` with `value`.
fn stray_key_reply(index: usize, name: &str, value: &str) -> String {
    let mut document: serde_json::Value = serde_json::from_str(&reply(&[])).unwrap();
    document["verdicts"][index][name] = value.into();
    document.to_string()
}

/// A complete true-verdict document whose verdict `index` lacks `field`.
fn missing_field_reply(index: usize, field: &str) -> String {
    let mut document: serde_json::Value = serde_json::from_str(&reply(&[])).unwrap();
    document["verdicts"][index]
        .as_object_mut()
        .unwrap()
        .remove(field);
    document.to_string()
}

/// A complete document with one false verdict whose verdict `index` lacks
/// `field`.
fn false_without(index: usize, field: &str) -> String {
    let ids = ["AC-WS-001", "G-WS-001"];
    let mut document: serde_json::Value = serde_json::from_str(&reply(&[ids[index]])).unwrap();
    document["verdicts"][index]
        .as_object_mut()
        .unwrap()
        .remove(field);
    document.to_string()
}

fn rejected_files(cwd: &Path) -> BTreeSet<String> {
    let rejected = cwd.join(".archon/lint-cache/fidelity/rejected");
    std::fs::read_dir(rejected)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn attempt_suffixes(files: &BTreeSet<String>) -> BTreeSet<usize> {
    files
        .iter()
        .map(|name| {
            name.rsplit_once("-attempt-")
                .and_then(|(_, n)| n.strip_suffix(".txt"))
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("unexpected rejected file {name}"))
        })
        .collect()
}

fn no_verdict_cached(cwd: &Path) {
    let cache = cwd.join(".archon/lint-cache/fidelity");
    let entries: Vec<_> = std::fs::read_dir(&cache)
        .expect("cache dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["rejected"], "no verdict is cached");
}

#[tokio::test]
async fn a_stray_key_reply_is_re_asked_with_its_parse_error_and_the_allowed_keys() {
    let temp = corpus();
    let stray = stray_key_reply(1, "weakest_task_text", "");
    let critic = FakeCritic::new(vec![Ok(stray.clone()), Ok(reply(&["G-WS-001"]))]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert!(
        evaluation
            .findings
            .iter()
            .any(|finding| finding.text.starts_with("obligation G-WS-001")),
        "the corrected reply is judged"
    );
    assert_eq!(critic.calls(), 2);
    let conversations = critic.conversations.lock().unwrap();
    let first = &conversations[0];
    let second = &conversations[1];
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 3, "{second:#?}");
    assert_eq!(second[0], first[0], "the question is asked unchanged");
    assert_eq!(second[1]["role"], "assistant");
    assert_eq!(second[1]["content"], stray.as_str());
    assert_eq!(second[2]["role"], "user");
    let feedback = second[2]["content"].as_str().unwrap();
    assert!(
        feedback.contains("verdicts[1]") && feedback.contains("unknown field `weakest_task_text`"),
        "the exact parse error with its path: {feedback}"
    );
    assert!(
        feedback.contains(&format!(
            "Each verdict has exactly these keys and no others: {}.",
            VERDICT_KEYS.join(", ")
        )),
        "{feedback}"
    );
    assert!(feedback.contains("exactly one key: verdicts"), "{feedback}");
    assert_eq!(
        attempt_suffixes(&rejected_files(temp.path())),
        BTreeSet::from([1])
    );
}

#[tokio::test]
async fn identical_bad_replies_are_operational_after_the_no_progress_window_not_after_two() {
    let temp = corpus();
    let attempts = 1 + FIDELITY_NO_PROGRESS_WINDOW;
    assert!(attempts > 2);
    let stray = stray_key_reply(1, "weakest_task_text", "");
    let critic = FakeCritic::new(vec![Ok(stray); attempts]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), attempts);
    let error = evaluation.operational_error().expect("operational");
    assert!(
        error.contains(&format!("after {attempts} attempts"))
            && error.contains("unknown field `weakest_task_text`")
            && error.contains("reply kept at"),
        "{error}"
    );
    no_verdict_cached(temp.path());
    assert_eq!(
        attempt_suffixes(&rejected_files(temp.path())),
        (1..=attempts).collect::<BTreeSet<_>>()
    );
}

/// Different bytes that fail in the same (verdict, kind) class are no progress.
#[tokio::test]
async fn the_same_error_from_different_bytes_is_no_progress() {
    let temp = corpus();
    let attempts = 1 + FIDELITY_NO_PROGRESS_WINDOW;
    let replies = (0..attempts)
        .map(|n| Ok(stray_key_reply(1, "weakest_task_text", &"x".repeat(n))))
        .collect();
    let critic = FakeCritic::new(replies);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), attempts);
    assert!(evaluation.operational_error().is_some());
    assert_eq!(rejected_files(temp.path()).len(), attempts);
}

#[tokio::test]
async fn bad_replies_with_new_errors_are_progress_and_keep_the_conversation_going() {
    let temp = corpus();
    // Each reply fails in a (verdict, kind) class no earlier reply had.
    let bad = vec![
        "not json".to_string(),
        stray_key_reply(0, "weakest_task_text", ""),
        stray_key_reply(1, "weakest_task_text", ""),
        r#"{"verdicts":[]}"#.to_string(),
        false_without(0, "quoted_task_text"),
        false_without(1, "weakest_task_id"),
    ];
    assert!(bad.len() > 1 + FIDELITY_NO_PROGRESS_WINDOW);
    let mut replies: Vec<_> = bad.iter().cloned().map(Ok).collect();
    replies.push(Ok(reply(&[])));
    let critic = FakeCritic::new(replies);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), bad.len() + 1);
    let conversations = critic.conversations.lock().unwrap();
    for (n, conversation) in conversations.iter().enumerate().skip(1) {
        assert_eq!(
            conversation.len(),
            3,
            "the question, the last rejected reply and its error; no more"
        );
        assert_eq!(conversation[0], conversations[0][0]);
        assert_eq!(conversation[1]["content"], bad[n - 1].as_str());
    }
    let feedback = conversations.last().unwrap()[2]["content"]
        .as_str()
        .unwrap();
    assert!(
        feedback.contains("G-WS-001") && feedback.contains("`weakest_task_id`"),
        "a false verdict's missing key is named for the critic to fix: {feedback}"
    );
    assert_eq!(
        attempt_suffixes(&rejected_files(temp.path())),
        (1..=bad.len()).collect::<BTreeSet<_>>()
    );
}

#[tokio::test]
async fn a_verdict_missing_a_required_field_is_never_accepted() {
    for field in ["obligation_id", "necessarily_true", "reason"] {
        let temp = corpus();
        let attempts = 1 + FIDELITY_NO_PROGRESS_WINDOW;
        let critic = FakeCritic::new(vec![Ok(missing_field_reply(0, field)); attempts]);
        let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
        assert_eq!(critic.calls(), attempts);
        let error = evaluation.operational_error().expect("never accepted");
        assert!(
            error.contains(&format!("missing field `{field}`")),
            "{error}"
        );
        no_verdict_cached(temp.path());
    }
}

/// New unknown-field names in the same verdict are one error kind there:
/// no progress, however the names (and so the byte offsets) differ.
#[tokio::test]
async fn new_unknown_field_names_in_the_same_verdict_stop_after_the_window() {
    let temp = corpus();
    let replies = (1..=10)
        .map(|n| Ok(stray_key_reply(1, &format!("zz_{}", "x".repeat(n)), "")))
        .collect();
    let critic = FakeCritic::new(replies);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), 1 + FIDELITY_NO_PROGRESS_WINDOW);
    let error = evaluation.operational_error().expect("operational");
    assert!(error.contains("unknown field `zz_xxxx`"), "{error}");
    assert_eq!(
        rejected_files(temp.path()).len(),
        1 + FIDELITY_NO_PROGRESS_WINDOW
    );
}

/// The base corpus with six more obligations claimed by its one task: one
/// call batch of eight, in sorted order. Returns the cwd and the ids.
fn wide_corpus() -> (tempfile::TempDir, Vec<String>) {
    let temp = corpus();
    let cwd = temp.path();
    let extra: Vec<String> = (1..=6).map(|n| format!("REQ-WS-{n:03}")).collect();
    let prd_path = cwd.join("tasks/PRD-WS-001.md");
    let mut prd = std::fs::read_to_string(&prd_path).unwrap();
    prd.push_str("\n## Requirements\n\n");
    for id in &extra {
        prd.push_str(&format!("- {id}: requirement {id}\n"));
    }
    std::fs::write(&prd_path, prd).unwrap();
    let task = cwd.join("tasks/PRD-WS-001/TASK-WS-001.md");
    let quoted: Vec<String> = extra.iter().map(|id| format!("\"{id}\"")).collect();
    let body = std::fs::read_to_string(&task).unwrap().replace(
        "implements: [\"G-WS-001\", \"AC-WS-001\"]",
        &format!(
            "implements: [\"G-WS-001\", \"AC-WS-001\", {}]",
            quoted.join(", ")
        ),
    );
    std::fs::write(&task, body).unwrap();
    let mut ids = vec!["AC-WS-001".to_string(), "G-WS-001".to_string()];
    ids.extend(extra);
    (temp, ids)
}

/// True verdicts for `ids`, with one stray key in verdict `stray_at`.
fn wide_reply(ids: &[String], stray_at: Option<usize>) -> String {
    let verdicts: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(n, id)| {
            let mut verdict = serde_json::json!({"obligation_id": id, "necessarily_true": true, "weakest_task_id": "", "reason": "obliged", "quoted_task_text": ""});
            if stray_at == Some(n) {
                verdict["zz_stray"] = "".into();
            }
            verdict
        })
        .collect();
    serde_json::json!({ "verdicts": verdicts }).to_string()
}

/// A reply that fixes the reported error in verdict 5 and shows one in
/// verdict 2 parsed less far, yet it is progress: that error is new there.
#[tokio::test]
async fn a_reply_fixing_a_later_verdict_with_an_error_in_an_earlier_one_is_progress() {
    let (temp, ids) = wide_corpus();
    let earlier = wide_reply(&ids, Some(2));
    let critic = FakeCritic::new(vec![
        Ok(wide_reply(&ids, Some(5))),
        Ok(earlier.clone()),
        Ok(earlier.clone()),
        Ok(earlier),
        Ok(wide_reply(&ids, None)),
    ]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 5);
}

/// Errors that move between verdicts are progress until every (verdict,
/// kind) pair has been seen; then the window ends the run.
#[tokio::test]
async fn errors_moving_between_verdicts_progress_until_the_pairs_run_out() {
    let (temp, ids) = wide_corpus();
    let mut replies: Vec<_> = (0..ids.len())
        .rev()
        .map(|n| Ok(wide_reply(&ids, Some(n))))
        .collect();
    let progress = replies.len();
    for n in 0..FIDELITY_NO_PROGRESS_WINDOW {
        replies.push(Ok(wide_reply(&ids, Some(n))));
    }
    let critic = FakeCritic::new(replies);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), progress + FIDELITY_NO_PROGRESS_WINDOW);
    assert!(evaluation.operational_error().is_some());
    assert_eq!(
        rejected_files(temp.path()).len(),
        progress + FIDELITY_NO_PROGRESS_WINDOW
    );
}
