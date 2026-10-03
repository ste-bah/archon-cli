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
