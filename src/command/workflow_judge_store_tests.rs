//! Saved judge verdicts are reused for an identical batch only (Issue 255).

use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use async_trait::async_trait;

use super::*;

/// Accepts every check, counting provider calls.
struct CountingJudge {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl WorkflowLlmClient for CountingJudge {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message(messages, system, tools, model).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, SeqCst);
        Ok(WorkflowAgentOutcome {
            content: r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"an invalid file","reason":"jq rejects it"}]}"#.into(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

fn contract(command: &str) -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "prds/PRD-X.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-X-001", "criterion": "output is valid",
            "check": {"kind": "command", "command": command, "cwd": "project_root"},
            "gap_permitted": false,
            "judgment": {"verdict": "refuted", "counterexample": "", "reason": "", "host_call_id": ""}
        }],
        "supplementary": []
    }))
    .unwrap()
}

#[tokio::test]
async fn an_identical_batch_reuses_the_saved_verdicts_and_any_other_is_judged() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let client = CountingJudge {
        calls: calls.clone(),
    };
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let store = || JudgeStore::at(dir.path().join("judge"));
    let first = store()
        .judge(&client, contract("jq -e '.valid' out.json"), &expected)
        .await
        .unwrap();
    assert_eq!(calls.load(SeqCst), 1);
    // A retry in a new process: a new store over the same directory.
    let again = store()
        .judge(&client, contract("jq -e '.valid' out.json"), &expected)
        .await
        .unwrap();
    assert_eq!(
        calls.load(SeqCst),
        1,
        "the identical batch is not asked again"
    );
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    // One changed byte of one check is a different input.
    store()
        .judge(
            &client,
            contract("jq -e '.valid == true' out.json"),
            &expected,
        )
        .await
        .unwrap();
    assert_eq!(calls.load(SeqCst), 2, "a changed batch is judged afresh");
}

#[tokio::test]
async fn an_unreadable_saved_verdict_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let client = CountingJudge {
        calls: calls.clone(),
    };
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let store = JudgeStore::at(dir.path().to_path_buf());
    store
        .judge(&client, contract("jq -e '.valid' out.json"), &expected)
        .await
        .unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        std::fs::write(entry.path(), b"{ truncated").unwrap();
    }
    store
        .judge(&client, contract("jq -e '.valid' out.json"), &expected)
        .await
        .unwrap();
    assert_eq!(calls.load(SeqCst), 2);
}

/// Round 2 (P1): the partial reply of a truncated judge is saved, so a
/// resumed freeze continues it instead of asking the judge again, and every
/// saved chunk counts as progress the executor can see.
#[tokio::test]
async fn a_resumed_judge_continues_from_the_saved_partial_reply() {
    use super::super::judge::continuation_tests::{HEAD, Scripted, TAIL};
    let dir = tempfile::tempdir().unwrap();
    let progress = Arc::new(crate::command::workflow_freeze_budget::FreezeProgress::default());
    let store = JudgeStore::at_with_progress(dir.path().to_path_buf(), progress.clone());
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let first = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("max_tokens"))),
    ]);
    let error = store
        .judge(&first, contract("jq -e . out.json"), &expected)
        .await
        .expect_err("the continuation stalled");
    assert!(super::super::judge::JudgeIncomplete::caused(&error).is_some());
    let after_first = progress.total();
    assert!(after_first >= 1, "the saved chunk is progress");

    let progress = Arc::new(crate::command::workflow_freeze_budget::FreezeProgress::default());
    let store = JudgeStore::at_with_progress(dir.path().to_path_buf(), progress.clone());
    let resumed = Scripted::new(vec![Ok((TAIL, Some("end_turn")))]);
    let judged = store
        .judge(&resumed, contract("jq -e . out.json"), &expected)
        .await
        .expect("the resumed judge completes the saved reply");
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
    let calls = resumed.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].len(), 3, "a continuation of the saved reply");
    assert_eq!(calls[0][1]["content"], HEAD);
    assert!(progress.total() > after_first, "progress never goes back");
}

/// The partial reply files the store holds.
fn partials(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("partial-") && name.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Round 3 (decision D): a reply the judge spends (complete but not usable)
/// keeps no progress credit: the host is shown only work that survives.
#[tokio::test]
async fn a_discarded_reply_keeps_no_progress_credit() {
    use super::super::judge::continuation_tests::{HEAD, Scripted};
    let dir = tempfile::tempdir().unwrap();
    let progress = Arc::new(crate::command::workflow_freeze_budget::FreezeProgress::default());
    let store = JudgeStore::at_with_progress(dir.path().to_path_buf(), progress.clone());
    let wrong_id = r#"epted","counterexample":"none","reason":"ok"},{"id":"AC-OTHER","verdict":"accepted","counterexample":"n","reason":"r"}]}"#;
    let mut replies = Vec::new();
    for _ in 0..3 {
        replies.push(Ok((HEAD, Some("max_tokens"))));
        replies.push(Ok((wrong_id, Some("end_turn"))));
    }
    let client = Scripted::new(replies);
    let error = store
        .judge(
            &client,
            contract("jq -e . out.json"),
            &BTreeSet::from(["AC-X-001".to_string()]),
        )
        .await
        .expect_err("no usable verdict");
    assert!(super::super::judge::JudgeIncomplete::caused(&error).is_some());
    assert_eq!(progress.total(), 0, "nothing usable survived");
}

/// Round 3 (decision D): saved counters are validated; a corrupt one is no
/// saved reply, never an overflow.
#[tokio::test]
async fn a_corrupt_partial_counter_is_ignored() {
    use super::super::judge::continuation_tests::{HEAD, Scripted};
    let dir = tempfile::tempdir().unwrap();
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let store = JudgeStore::at(dir.path().to_path_buf());
    let first = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("max_tokens"))),
    ]);
    let _ = store
        .judge(&first, contract("jq -e . out.json"), &expected)
        .await;
    let path = partials(dir.path())
        .pop()
        .expect("a partial reply was saved");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["chunks"] = serde_json::json!(u64::MAX);
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();

    let progress = Arc::new(crate::command::workflow_freeze_budget::FreezeProgress::default());
    let store = JudgeStore::at_with_progress(dir.path().to_path_buf(), progress.clone());
    let fresh = Scripted::new(vec![Ok((
        r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"ok"}]}"#,
        Some("end_turn"),
    ))]);
    store
        .judge(&fresh, contract("jq -e . out.json"), &expected)
        .await
        .expect("a fresh ask answers");
    assert_eq!(
        fresh.calls()[0].len(),
        1,
        "the corrupt reply was not continued"
    );
    assert!(progress.total() < 10, "{}", progress.total());
}

/// Round 3 (decision D): the saved partial reply is redacted and private.
#[tokio::test]
async fn the_saved_partial_reply_is_redacted_and_private() {
    use super::super::judge::continuation_tests::Scripted;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("judge");
    let store = JudgeStore::at(dir.clone());
    let leaky = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"uses sk-abcdefghijklmnopqrstuvwxyz0123 as"#;
    let client = Scripted::new(vec![
        Ok((leaky, Some("max_tokens"))),
        Ok(("", Some("max_tokens"))),
    ]);
    let _ = store
        .judge(
            &client,
            contract("jq -e . out.json"),
            &BTreeSet::from(["AC-X-001".to_string()]),
        )
        .await;
    let path = partials(&dir).pop().expect("a partial reply was saved");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("sk-abcdefghijklmnopqrstuvwxyz0123"),
        "{text}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "{dir_mode:o}");
    }
}

#[tokio::test]
async fn failed_verdict_persistence_keeps_the_continuable_document() {
    use super::super::judge::continuation_tests::{HEAD, Scripted, TAIL};
    let temp = tempfile::tempdir().unwrap();
    let progress = Arc::new(FreezeProgress::default());
    let store = JudgeStore::at_with_progress(temp.path().to_path_buf(), progress.clone());
    let subset = contract("check");
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let client = Scripted::new(vec![Ok((TAIL, Some("end_turn")))]);
    let key = JudgeStore::key(&client, &subset, &expected).unwrap();
    let first = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("max_tokens"))),
    ]);
    assert!(
        store
            .judge(&first, subset.clone(), &expected)
            .await
            .is_err()
    );
    std::fs::create_dir(temp.path().join(format!("{key}.json"))).unwrap();
    store.judge(&client, subset, &expected).await.unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(partials(temp.path()).pop().unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        saved["reply"], HEAD,
        "verdict save failure must preserve continuation"
    );
    assert_eq!(saved["completed"], false);
}

#[tokio::test]
async fn orphan_completion_marker_is_not_progress() {
    use super::super::judge::continuation_tests::{HEAD, Scripted};
    let temp = tempfile::tempdir().unwrap();
    let progress = Arc::new(FreezeProgress::default());
    let subset = contract("check");
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let client = Scripted::new(vec![Ok(("", Some("max_tokens"))); 3]);
    let key = JudgeStore::key(&client, &subset, &expected).unwrap();
    let initial = JudgeStore::at_with_progress(temp.path().to_path_buf(), progress.clone());
    let first = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("max_tokens"))),
    ]);
    assert!(
        initial
            .judge(&first, subset.clone(), &expected)
            .await
            .is_err()
    );
    PartialReply::new(temp.path(), &key, &progress).completed();
    let resumed = Arc::new(FreezeProgress::default());
    let store = JudgeStore::at_with_progress(temp.path().to_path_buf(), resumed.clone());
    assert!(store.judge(&client, subset, &expected).await.is_err());
    assert_eq!(resumed.total(), 0, "a marker needs saved verdicts");
}
