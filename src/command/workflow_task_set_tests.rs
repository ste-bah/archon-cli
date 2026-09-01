use std::sync::Arc;

use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptancePin,
};
use async_trait::async_trait;

use super::*;

#[derive(Clone)]
struct JudgeClient {
    result: Result<String, String>,
}

#[async_trait]
impl WorkflowLlmClient for JudgeClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert!(tools.is_empty());
        assert_eq!(model, "sonnet");
        assert_eq!(
            messages.len(),
            1,
            "judge needs one provider-valid user message"
        );
        assert_eq!(messages[0]["role"], "user");
        assert!(
            messages[0]["content"]
                .as_str()
                .is_some_and(|text| { text.contains("exactly one decision for every input id") })
        );
        assert!(system.iter().all(|entry| {
            entry["text"]
                .as_str()
                .is_some_and(|text| !text.contains("acceptance contract JSON"))
        }));
        match &self.result {
            Ok(content) => Ok(WorkflowAgentOutcome {
                content: content.clone(),
                stop_reason: Some("end_turn".into()),
                ..WorkflowAgentOutcome::default()
            }),
            Err(message) => Err(WorkflowError::port(std::io::Error::other(message.clone()))),
        }
    }
}

fn seed(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf, Vec<u8>) {
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = temp.path().join("prds/PRD-X.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n",
    )
    .unwrap();
    let contract = br#"{
      "schema_version":1,
      "prd":{"path":"prds/PRD-X.md","digest":"pending"},
      "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
      "acceptance":[{
        "id":"AC-X-001","criterion":"untrusted draft summary",
        "check":{"kind":"command","command":"jq -e '.valid == true' out.json","cwd":"project_root"},
        "gap_permitted":false,
        "judgment":{"verdict":"refuted","counterexample":"untrusted","reason":"untrusted","host_call_id":"untrusted"}
      }],
      "supplementary":[]
    }"#
    .to_vec();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &contract).unwrap();
    (tasks, prd, contract)
}

#[tokio::test]
async fn judge_error_leaves_no_partial_freeze() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let error = freeze_acceptance(
        temp.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Err("invalid x-api-key".into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("invalid x-api-key"), "{error}");
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
}

#[tokio::test]
async fn malformed_judgment_leaves_no_partial_freeze() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    assert!(
        prepare_acceptance_freeze(
            temp.path(),
            &tasks,
            &prd,
            archon_core::config::GateMode::Observe,
            Arc::new(JudgeClient {
                result: Ok("not json".into()),
            }),
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
}

#[tokio::test]
async fn refuting_judgment_is_an_observe_policy_finding_and_enforce_blocker() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let response = r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"a passing false state","reason":"weak"}]}"#;
    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap();
    assert!(
        prepared
            .findings
            .iter()
            .any(|finding| finding.text.contains("refuted"))
    );
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )
    .unwrap();
    let permit = disposition.take_publication_permit().unwrap();
    publish_acceptance_freeze(prepared, permit).unwrap();
    assert!(tasks.join(ACCEPTANCE_LOCK_FILE).is_file());

    let second = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&second);
    let error = freeze_acceptance(
        second.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("refuted"), "{error}");
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
}

#[tokio::test]
async fn accepted_judgment_publishes_contract_lock_and_pin_together() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let result = freeze_acceptance(
        temp.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted false state","reason":"command rejects it"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    assert_ne!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(tasks.join(ACCEPTANCE_LOCK_FILE).is_file());
    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(pin.acceptance_digest, result.acceptance_digest);
    assert!(result.freeze_event_id.starts_with("acceptance-freeze-"));
    let frozen = std::fs::read_to_string(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    assert!(
        frozen.contains("acceptance-judge-batch:AC-X-001"),
        "{frozen}"
    );
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_str(&frozen).unwrap();
    assert_eq!(contract.acceptance[0].criterion, "output is valid");
    assert_eq!(
        contract.gap_policy.required_fields,
        archon_workflow::task_set_contract::REQUIRED_RESIDUAL_GAP_FIELDS
            .iter()
            .map(|field| (*field).to_string())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn publication_error_preserves_the_original_contract_and_pin() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    std::fs::create_dir(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap();
    let error = freeze_acceptance(
        temp.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted false state","reason":"command rejects it"}]}"#.into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains(ACCEPTANCE_LOCK_FILE), "{error}");
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
}

fn seed_frozen_acceptance(temp: &tempfile::TempDir) -> (std::path::PathBuf, AcceptancePin) {
    let (tasks, prd, _) = seed(temp);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        freeze_acceptance(
            temp.path(),
            &tasks,
            &prd,
            Arc::new(JudgeClient {
                result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted false state","reason":"command rejects it"}]}"#.into()),
            }),
        )
        .await
        .unwrap();
    });
    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    (tasks, pin)
}

#[path = "workflow_task_set_skeleton_tests.rs"]
mod skeleton_tests;

#[derive(Clone)]
struct FinishReasonJudge {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    content: String,
    stop_reason: Option<String>,
}

#[async_trait]
impl WorkflowLlmClient for FinishReasonJudge {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: self.content.clone(),
            stop_reason: self.stop_reason.clone(),
            ..WorkflowAgentOutcome::default()
        })
    }
}

#[tokio::test]
async fn batched_judge_runs_once_and_truncation_is_refused_before_parse() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let error = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(FinishReasonJudge {
            calls: calls.clone(),
            content: "{ definitely incomplete".into(),
            stop_reason: Some("max_tokens".into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        error.contains("truncated") && error.contains("never repaired"),
        "{error}"
    );
    assert!(
        !error.contains("malformed batched JSON"),
        "finish reason must win: {error}"
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
}

#[tokio::test]
async fn observe_freeze_stamps_policy_findings_and_enforce_requires_refreeze() {
    use archon_workflow::task_set_contract::{AcceptanceLock, FreezeGateMode, TASK_SKELETON_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let draft = std::fs::read_to_string(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let weak = draft.replace("jq -e '.valid == true' out.json", "true");
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), weak).unwrap();
    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"accepted for shadow measurement"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    // Two findings, not one: the floor defect, plus the contradiction of a
    // judge returning `accepted` for the very criterion the policy layer
    // reports. A verdict cannot outrank a defect the host checked itself.
    assert_eq!(prepared.findings.len(), 2, "{:?}", prepared.findings);
    assert!(
        prepared
            .findings
            .iter()
            .any(|finding| finding.text.contains("contradicts a finding the host verified")),
        "the judge/policy disagreement must itself be a finding: {:?}",
        prepared.findings
    );
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )
    .unwrap();
    let permit = disposition.take_publication_permit().unwrap();
    publish_acceptance_freeze(prepared, permit).unwrap();
    let lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap()).unwrap();
    assert_eq!(lock.gate.mode, FreezeGateMode::Observe);
    // Two: the floor defect and the judge contradicting it.
    assert_eq!(lock.gate.finding_count, 2);
    assert!(!lock.gate.findings_digest.is_empty());

    std::fs::write(
        tasks.join(TASK_SKELETON_FILE),
        r#"{"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]}]}"#,
    )
    .unwrap();
    let prepared = prepare_skeleton_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    let finding = prepared
        .findings
        .iter()
        .find(|finding| finding.text.contains("predecessor acceptance freeze"))
        .expect("predecessor finding");
    assert!(
        finding.text.contains("re-freeze under enforce"),
        "{}",
        finding.text
    );
}

#[path = "workflow_task_set_review_tests.rs"]
mod review_tests;

#[path = "workflow_task_set_republish_tests.rs"]
mod republish_tests;

#[tokio::test]
async fn acceptance_candidate_prepares_exact_staged_bytes_without_reading_or_writing_live_contract()
{
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    std::fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        b"malformed live sentinel",
    )
    .unwrap();
    let response = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"rejects"}]}"#;

    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        candidate,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap();
    let (evaluation, outputs) = prepared.into_staged_parts();

    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        b"malformed live sentinel"
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
    assert!(evaluation.findings.is_empty());
    assert_eq!(
        outputs
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        [
            "acceptance-contract.json",
            "acceptance-contract.lock",
            "acceptance-pin.json"
        ]
    );
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&outputs[0].1).unwrap();
    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
}
