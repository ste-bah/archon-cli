use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};
use archon_workflow::task_set_contract::{AcceptanceContract, JudgeDecision};
use async_trait::async_trait;

use super::*;

struct QuotedJudge {
    verdict: &'static str,
    counterexample: &'static str,
}

#[async_trait]
impl WorkflowLlmClient for QuotedJudge {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let prompt = messages[0]["content"].as_str().unwrap();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(prompt.split_once("Checks: ").unwrap().1).unwrap();
        let decisions: Vec<_> = checks
            .iter()
            .map(|check| {
                serde_json::json!({
                    "id": check["id"],
                    "verdict": self.verdict,
                    "counterexample": self.counterexample,
                    "reason": "the quoted output rule refuses the check's setup"
                })
            })
            .collect();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": decisions}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("passability uses the sampled judge")
    }
}

fn fixture(output: &[u8]) -> (AcceptanceContract, BaselineRuns) {
    use crate::command::workflow_task_set::executability::CheckEvidence;
    use archon_workflow::acceptance_scratch::CheckResult;

    let mut contract = crate::command::workflow_task_set::judge::continuation_tests::contract();
    contract.acceptance[0].id = "AC-X-001".into();
    contract.acceptance[0].judgment.verdict = JudgeDecision::Accepted;
    let result = CheckResult {
        fault: None,
        acceptance_id: "AC-X-001".into(),
        environment_note: None,
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: output.to_vec(),
        stderr: Vec::new(),
        operational_error: None,
        classification: None,
    };
    let runs = BaselineRuns {
        reuse: Default::default(),
        commit: "baseline-x".into(),
        failures: BTreeMap::from([(
            "AC-X-001".into(),
            CheckEvidence {
                result,
                copy_layout: None,
            },
        )]),
        environment: Default::default(),
        forwarded: Vec::new(),
    };
    (contract, runs)
}

async fn judge(
    output: &[u8],
    counterexample: &'static str,
) -> (BTreeMap<String, String>, Option<serde_json::Value>) {
    let (contract, runs) = fixture(output);
    let client = QuotedJudge {
        verdict: "refuted",
        counterexample,
    };
    let audit = tempfile::tempdir().unwrap();
    let findings = cannot_pass_findings(
        &client,
        "reviewer",
        audit.path(),
        audit.path(),
        &contract,
        &BTreeSet::from(["AC-X-001".into()]),
        &runs,
        "",
        Some(audit.path()),
    )
    .await
    .unwrap();
    let record = std::fs::read_dir(audit.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| std::fs::read(entry.path()).unwrap())
        .find_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(|value| value.get("check_id").is_some());
    (findings, record)
}

#[tokio::test]
async fn fabricated_quote_is_accepted_without_a_finding_and_records_the_missing_text() {
    let (findings, record) = judge(
        b"only the missing feature is described",
        "the rule says 'minimum 400 rows'",
    )
    .await;
    assert!(
        findings.is_empty(),
        "ungrounded quote must not become a finding: {findings:?}"
    );
    let reason = record.unwrap()["reason"].as_str().unwrap().to_string();
    assert!(
        reason.contains("minimum 400 rows") && reason.contains("output does not contain"),
        "missing quote reason: {reason}"
    );
}

#[tokio::test]
async fn a_present_quote_keeps_the_refutation_and_finding() {
    let (fabricated, _) = judge(
        b"only the missing feature is described",
        "the rule says 'minimum 400 rows'",
    )
    .await;
    assert!(
        fabricated.is_empty(),
        "fabricated-quote control must be accepted: {fabricated:?}"
    );
    let (findings, _) = judge(
        b"the rule says minimum 400 rows",
        "the check's setup violates 'minimum 400 rows'",
    )
    .await;
    assert!(findings["AC-X-001"].contains("cannot pass as written"));
    assert!(findings["AC-X-001"].contains("minimum 400 rows"));
}

#[tokio::test]
async fn every_part_of_an_elided_quote_must_be_present() {
    let (missing_part, _) = judge(
        b"minimum 400 rows are required",
        "the rule says 'minimum ... rejected rows'",
    )
    .await;
    assert!(
        missing_part.is_empty(),
        "an absent elided part must be accepted: {missing_part:?}"
    );
    let (findings, _) = judge(
        b"minimum 400 rows are required",
        "the rule says 'minimum ... 400 rows'",
    )
    .await;
    assert!(findings.contains_key("AC-X-001"));
}

#[tokio::test]
async fn a_refutation_without_a_quoted_fragment_is_accepted() {
    let (findings, record) = judge(b"some output", "the output proves the setup is refused").await;
    assert!(
        findings.is_empty(),
        "unquoted refutation must not become a finding: {findings:?}"
    );
    assert!(
        record.unwrap()["reason"]
            .as_str()
            .unwrap()
            .contains("no quoted fragment")
    );
}

#[tokio::test]
async fn stall_rejudge_uses_the_same_grounding_before_returning_findings() {
    // The stalled live path calls cannot_pass_findings for both current and
    // base-tree evidence; this is that exact production entry point.
    let (findings, _) = judge(
        b"only the missing feature is described",
        "the setup is refused",
    )
    .await;
    assert!(
        findings.is_empty(),
        "stall re-judge must not return an ungrounded finding: {findings:?}"
    );
}

#[tokio::test]
async fn text_beyond_the_bounded_output_cannot_ground_a_quote() {
    let middle = (0..200)
        .map(|index| {
            if index == 100 {
                "the rule says hidden refusal rule\n".to_string()
            } else {
                format!("ordinary output line number {index:03} with some filler text\n")
            }
        })
        .collect::<String>();
    let output = middle.into_bytes();
    let (findings, _) = judge(&output, "the rule says 'hidden refusal rule'").await;
    assert!(
        findings.is_empty(),
        "text beyond judge evidence must not ground a quote: {findings:?}"
    );
}

#[tokio::test]
async fn quote_matching_ignores_case_and_collapses_whitespace() {
    let (different_text, _) = judge(
        b"RULE: MINIMUM 400 ROWS",
        "the rule says 'minimum 500 rows'",
    )
    .await;
    assert!(
        different_text.is_empty(),
        "a different rule must be accepted: {different_text:?}"
    );
    let (findings, _) = judge(
        b"RULE: MINIMUM   400\nROWS",
        "the rule says ‘minimum 400 rows’",
    )
    .await;
    assert!(findings.contains_key("AC-X-001"));
}

#[tokio::test]
async fn a_plural_possessive_after_a_grounded_quote_keeps_the_refutation() {
    let (findings, _) = judge(
        b"minimum 400 rows",
        "the rule says 'minimum 400 rows'; the tests' names repeat 'minimum 400 rows'",
    )
    .await;
    assert!(
        findings.contains_key("AC-X-001"),
        "a plural possessive must not open a bogus quote: {findings:?}"
    );
}
