//! The re-ask conversation: a rejected reply goes back to the critic with
//! the exact parse error and the allowed keys, and the critic is asked again
//! while its replies make progress. A reply byte-identical to an earlier one,
//! or one that fails with an error an earlier one had, is no progress; the
//! audit is operational once [`FIDELITY_NO_PROGRESS_WINDOW`] of them come in
//! a row. A verdict with a missing or unknown field is never accepted.

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

/// Different bytes that fail with the same error are no progress: the
/// location of the error inside the reply is not part of the error.
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
    let bad = vec![
        "not json".to_string(),
        r#"{"verdicts":[]}"#.to_string(),
        stray_key_reply(0, "weakest_task_text", ""),
        stray_key_reply(1, "weakest_task_text", ""),
        missing_field_reply(0, "reason"),
        missing_field_reply(1, "necessarily_true"),
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
    let last = conversations.last().unwrap();
    assert_eq!(last.len(), 1 + 2 * bad.len(), "every rejected reply stays");
    for (n, reply) in bad.iter().enumerate() {
        assert_eq!(last[1 + 2 * n]["content"], reply.as_str());
    }
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
