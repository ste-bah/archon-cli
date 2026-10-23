use std::sync::Arc;

use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptancePin,
};
use async_trait::async_trait;

use super::*;

#[path = "workflow_task_set_cas_tests.rs"]
mod cas_tests;
#[path = "workflow_task_set_refuted_tests.rs"]
mod refuted_tests;
#[path = "workflow_task_set_scope_tests.rs"]
mod scope_tests;

#[path = "workflow_task_set_test_client.rs"]
mod test_client;
use test_client::JudgeClient;

fn publish_observed(
    prepared: PreparedAcceptanceFreeze,
    project: &Path,
) -> Result<FreezeAcceptanceResult> {
    let findings = prepared.findings.clone();
    let identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        project,
        GateMode::Observe,
        GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )?;
    let permit = disposition.take_publication_permit().unwrap();
    publish_acceptance_freeze(prepared, permit)
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
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
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
async fn refuting_judgment_is_never_published_in_observe_or_enforce() {
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
    // Observe lets the gate grant a permit, but a refuted check is never
    // published in any mode: it can never run.
    let permit = disposition.take_publication_permit().unwrap();
    let error = publish_acceptance_freeze(prepared, permit)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not accepted by the judge"), "{error}");
    assert!(error.contains("AC-X-001"), "{error}");
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());

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
    assert_eq!(result.candidate_digest, content_digest(&original));
    assert_eq!(result.live_contract_digest, Some(content_digest(&original)));
    assert_eq!(
        result.acceptance_digest,
        content_digest(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
    );
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
async fn whole_set_freeze_prepares_from_the_current_contract_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut candidate: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    let repaired_command = "jq -e '.valid == true and .repaired == true' out.json";
    candidate["acceptance"][0]["check"]["command"] = repaired_command.into();
    let current = serde_json::to_vec_pretty(&candidate).unwrap();
    std::fs::write(&contract_path, &current).unwrap();
    std::fs::write(
        temp.path().join("out.json"),
        r#"{"valid":true,"repaired":true}"#,
    )
    .unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());

    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"the false state is rejected","reason":"the command checks both fields"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    let prepared_contract = prepared.contract().unwrap();
    let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &prepared_contract.acceptance[0].check
    else {
        panic!("the current entry remains a command check");
    };
    assert_eq!(command, repaired_command);
    assert_eq!(prepared.candidate_digest, content_digest(&current));
    assert_eq!(
        prepared.live_contract_digest,
        Some(content_digest(&current))
    );
}

#[tokio::test]
async fn a_first_whole_set_draft_publishes_when_no_contract_exists() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    std::fs::remove_file(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    std::fs::write(temp.path().join("out.json"), r#"{"valid":true}"#).unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze_from_candidate(
        temp.path(), &tasks, &prd, GateMode::Observe, candidate.clone(),
        Arc::new(JudgeClient { result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"false is rejected","reason":"checks validity"}]}"#.into()) }),
    ).await.unwrap();
    assert_eq!(prepared.live_contract_digest, None);
    assert_eq!(prepared.result.candidate_digest, content_digest(&candidate));
    let result = publish_observed(prepared, temp.path()).unwrap();
    assert_eq!(result.live_contract_digest, None);
    assert_eq!(
        result.acceptance_digest,
        content_digest(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
    );
}

#[tokio::test]
async fn contract_changed_after_prepare_rebuilds_without_overwriting_the_new_contract() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    std::fs::write(temp.path().join("out.json"), r#"{"valid":true}"#).unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze_from_candidate(
        temp.path(), &tasks, &prd, GateMode::Observe, original.clone(),
        Arc::new(JudgeClient { result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"false is rejected","reason":"checks validity"}]}"#.into()) }),
    ).await.unwrap();
    let mut newer_value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    newer_value["acceptance"][0]["check"]["command"] =
        "jq -e '.valid == true and .repaired == true' out.json".into();
    let newer = serde_json::to_vec_pretty(&newer_value).unwrap();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &newer).unwrap();
    let error =
        publish_observed(prepared, temp.path()).expect_err("stale prepared snapshot retries");
    assert!(
        error.downcast_ref::<StaleAcceptanceContract>().is_some(),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        newer,
        "the stale prepared candidate did not overwrite the new contract"
    );
    std::fs::write(
        temp.path().join("out.json"),
        r#"{"valid":true,"repaired":true}"#,
    )
    .unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let rebuilt = crate::command::workflow_task_set::prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        newer.clone(),
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"false is rejected","reason":"checks validity and repair"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(rebuilt.live_contract_digest, Some(content_digest(&newer)));
    publish_observed(rebuilt, temp.path()).expect("the rebuilt snapshot publishes");
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
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        self.send_message(messages, system, tools, model).await
    }

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

#[path = "workflow_task_set_truncation_tests.rs"]
mod truncation_tests;

#[path = "workflow_task_set_policy_tests.rs"]
mod policy_tests;

#[path = "workflow_task_set_review_tests.rs"]
mod review_tests;

#[path = "workflow_task_set_republish_tests.rs"]
mod republish_tests;

#[path = "workflow_task_set_candidate_tests.rs"]
mod candidate_tests;

#[path = "workflow_acceptance_preflight_tests.rs"]
mod acceptance_preflight_tests;

#[tokio::test]
async fn per_entry_candidate_reaches_real_preflight_and_judge() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let contract: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let candidate =
        serde_json::to_vec(&serde_json::json!({"entries":contract["acceptance"]})).unwrap();
    let prepared = prepare_acceptance_freeze_from_candidate(temp.path(), &tasks, &prd,
        GateMode::Observe, candidate, Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"checked"}]}"#.into())
        })).await.expect("entry envelope reaches the actual freeze path");
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original,
        "preparation must not publish"
    );
}

#[path = "workflow_judge_reuse_tests.rs"]
mod judge_reuse_tests;

#[path = "workflow_task_set_absent_contract_tests.rs"]
mod absent_contract_tests;
