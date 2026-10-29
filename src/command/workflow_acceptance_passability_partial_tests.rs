//! A passability judge that decides some checks of its batch and then
//! cannot complete keeps those verdicts: only the undecided checks stay
//! pending for the retry (Issue 497).

use std::sync::Mutex;

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use async_trait::async_trait;

use super::*;
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

/// Decides the first check of every batch it is shown that holds it, and
/// never the second: its replies leave that one without a decision.
#[derive(Default)]
struct DecidesOnlyTheFirst {
    batches: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl WorkflowLlmClient for DecidesOnlyTheFirst {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let prompt = messages[0]["content"].as_str().unwrap_or_default();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(prompt.split_once("Checks: ").expect("checks").1)
                .expect("checks are JSON");
        let ids: Vec<String> = (checks.iter())
            .filter_map(|check| check["id"].as_str().map(str::to_string))
            .collect();
        self.batches.lock().unwrap().push(ids.clone());
        let decisions: Vec<_> = (ids.iter())
            .filter(|id| id.as_str() == "AC-X-001")
            .map(|id| {
                serde_json::json!({"id": id, "verdict": "refuted", "counterexample": "the output says \"refused\"", "reason": "the output states the refusing rule"})
            })
            .collect();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({ "decisions": decisions }).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the judge always samples explicitly")
    }
}

fn candidate(id: &str) -> Candidate {
    let evidence = Evidence {
        commit: "base".into(),
        exit_code: Some(1),
        stderr: "refused".into(),
        stdout: String::new(),
        reused: false,
        reuse_why: None,
        evidence_reference: None,
    };
    Candidate {
        id: id.into(),
        shown: serde_json::json!({"id": id, "criterion": "c", "baseline": evidence}),
        evidence,
        key: format!("key-{id}"),
    }
}

#[tokio::test]
async fn an_incomplete_batch_delivers_refutations_and_leaves_undecided_checks_uncached() {
    let mut contract = super::super::judge::continuation_tests::contract();
    let mut second = contract.acceptance[0].clone();
    second.id = "AC-X-002".into();
    contract.acceptance.push(second);
    let candidates = [candidate("AC-X-001"), candidate("AC-X-002")];
    let dir = tempfile::tempdir().unwrap();
    let store = Store {
        dir: dir.path().join("passability"),
    };
    let resume = FreezeResume::saving(FreezeBudget::unlimited(), true);
    let client = DecidesOnlyTheFirst::default();
    let refuted = assess(
        &client,
        "sonnet",
        &contract,
        &candidates,
        &Redactor::from_vars(Vec::new(), &[]),
        Some(&store),
        &resume,
        None,
    )
    .await
    .expect("a partial refutation is already actionable");
    assert!(refuted.contains_key("AC-X-001"));
    assert!(!refuted.contains_key("AC-X-002"));
    let deferred: Vec<_> = candidates
        .iter()
        .filter(|candidate| !refuted.contains_key(&candidate.id))
        .map(|candidate| candidate.id.clone())
        .collect();
    assert_eq!(
        deferred,
        ["AC-X-002".to_string()],
        "a decided check is not pending"
    );
    let saved = store
        .load("key-AC-X-001")
        .expect("the decided verdict is saved");
    assert_eq!(saved.verdict, JudgeDecision::Refuted);
    assert!(store.load("key-AC-X-002").is_none());
    assert_eq!(
        client.batches.lock().unwrap()[0],
        ["AC-X-001".to_string(), "AC-X-002".to_string()],
        "both were asked"
    );
}
