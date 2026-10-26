use super::*;
use crate::command::workflow_task_set::judge::continuation_tests::contract;
use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use async_trait::async_trait;
use std::sync::Mutex;

#[derive(Default)]
struct Scripted(Mutex<Vec<Vec<serde_json::Value>>>);

impl Scripted {
    fn new() -> Self {
        Self::default()
    }

    fn calls(&self) -> Vec<Vec<serde_json::Value>> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait]
impl WorkflowLlmClient for Scripted {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let mut calls = self.0.lock().unwrap();
        let call_index = calls.len();
        calls.push(messages.clone());
        drop(calls);
        let prompt = messages[0]["content"].as_str().unwrap_or_default();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(prompt.split_once("Checks: ").expect("checks").1)?;
        let wanted = if call_index == 0 {
            "AC-X-001"
        } else {
            "AC-X-002"
        };
        let decisions = checks
            .iter()
            .filter(|check| check["id"] == wanted)
            .map(|check| {
                serde_json::json!({
                    "id": check["id"], "verdict": "accepted", "counterexample": "none",
                    "reason": "the neutral check is sound"
                })
            })
            .collect::<Vec<_>>();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": decisions}).to_string(),
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
        unreachable!("judge samples with explicit temperature")
    }
}

#[test]
fn toolchain_invocation_errors_refute_and_instruction_changes_rekey_verdicts() {
    assert!(INSTRUCTION.contains("toolchain program"));
    assert!(INSTRUCTION.contains("shows which program rejected the invocation"));
    assert!(INSTRUCTION.contains("cannot pass as written"));
    assert!(INSTRUCTION.contains("feature is absent stays accepted"));
    let input = serde_json::json!({"id": "AC-X-001", "exit_code": 1});
    assert_ne!(
        verdict_key(INSTRUCTION, input.clone()),
        verdict_key("changed instruction", input),
        "the instruction is part of the verdict cache key"
    );
}

#[test]
fn production_key_covers_the_full_baseline_evidence() {
    let client = Scripted::new();
    let entry = contract().acceptance.remove(0);
    let requirements = BTreeMap::new();
    let evidence = Evidence {
        commit: "base".into(),
        exit_code: Some(1),
        stderr: "error: unexpected argument '--exact' found".into(),
        stdout: String::new(),
        reused: false,
        reuse_why: None,
        evidence_reference: None,
    };
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: entry.id.clone(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: evidence.stderr.as_bytes().to_vec(),
        environment_note: None,
        operational_error: None,
        fault: None,
        classification: None,
    };
    let first = super::key(
        &client,
        "sonnet",
        &serde_json::json!({"id": entry.id, "baseline": evidence}),
        &entry,
        &requirements,
        Some("probe-key"),
        &result,
    );
    let mut changed = result.clone();
    changed.stderr = b"error: unknown option '--exact'".to_vec();
    let second = super::key(
        &client,
        "sonnet",
        &serde_json::json!({"id": entry.id, "baseline": {"stderr": "error: unknown option '--exact'"}}),
        &entry,
        &requirements,
        Some("probe-key"),
        &changed,
    );
    assert_ne!(
        first, second,
        "production passability key includes evidence"
    );
}

#[test]
fn instruction_narrows_toolchain_refutation_and_preserves_missing_file_carveout() {
    assert!(INSTRUCTION.contains("only when the output shows which program rejected"));
    assert!(INSTRUCTION.contains("when it cannot tell, accept"));
    assert!(INSTRUCTION.contains("missing file, target, test or module"));
    assert!(INSTRUCTION.contains("asks to add or change"));
}

fn candidate(id: &str) -> Candidate {
    let evidence = Evidence {
        commit: "base".into(),
        exit_code: Some(1),
        stderr: "baseline output".into(),
        stdout: String::new(),
        reused: false,
        reuse_why: None,
        evidence_reference: None,
    };
    Candidate {
        id: id.into(),
        shown: serde_json::json!({"id": id, "criterion": "A check has a neutral property.", "baseline": evidence}),
        evidence,
        key: format!("key-{id}"),
    }
}

#[tokio::test]
async fn a_missing_passability_id_is_reasked_with_the_passability_prompt() {
    let mut subject = contract();
    let mut second = subject.acceptance[0].clone();
    second.id = "AC-X-002".into();
    subject.acceptance.push(second);
    let candidates = [candidate("AC-X-001"), candidate("AC-X-002")];
    let client = Scripted::new();

    ask(
        &client,
        "sonnet",
        &subject,
        &[&candidates[0], &candidates[1]],
    )
    .await
    .expect("both checks receive decisions");

    let calls = client.calls();
    let reask = calls[1][0]["content"].as_str().expect("user prompt");
    assert!(
        reask.contains("Each acceptance check below already ran once"),
        "missing passability prompt sentence: {reask}"
    );
    assert!(
        !reask.contains("Judge supplied named_files content as written"),
        "acceptance judge prompt leaked into passability re-ask: {reask}"
    );
    assert!(
        reask.contains("AC-X-002"),
        "re-ask must contain only the missing id: {reask}"
    );
    assert!(
        !reask.contains("AC-X-001"),
        "decided id must not be re-asked: {reask}"
    );
}

#[tokio::test]
async fn ambiguous_clap_style_evidence_is_included_verbatim_in_the_judge_prompt() {
    let subject = contract();
    let mut check = candidate("AC-X-001");
    check.evidence.stderr = "error: unexpected argument '--exact' found".into();
    check.shown["baseline"]["stderr"] = check.evidence.stderr.clone().into();
    let client = Scripted::new();
    ask(&client, "sonnet", &subject, &[&check])
        .await
        .expect("the tool-neutral evidence shape is presented to the judge");
    let prompt = client.calls()[0][0]["content"].as_str().unwrap().to_owned();
    assert!(
        prompt.contains("error: unexpected argument '--exact' found"),
        "replay evidence was absent from the prompt: {prompt}"
    );
}
