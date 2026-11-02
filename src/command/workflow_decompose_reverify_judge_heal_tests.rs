use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct RecoveringJudge(AtomicUsize);

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for RecoveringJudge {
    async fn send_message_with_temperature(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        if self.0.fetch_add(1, Ordering::SeqCst) < 3 {
            // Advance only the existing judge window; resume before the next
            // real check-copy process starts.
            struct ClockGuard;
            impl Drop for ClockGuard {
                fn drop(&mut self) {
                    tokio::time::resume();
                }
            }
            tokio::time::pause();
            let _clock = ClockGuard;
            std::future::pending::<()>().await;
            unreachable!();
        }
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::json!({"decisions":[{"id":"AC-X-001", "verdict":"accepted",
                "counterexample":"no setup rule refused the check", "reason":"the required fixture is absent"}]}).to_string(),
            stop_reason: Some("end_turn".into()), ..Default::default()
        })
    }
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        unreachable!("judge sampling must use the real sampled dispatch")
    }
}

#[tokio::test]
async fn reverify_acceptance_recovers_interrupted_judge_io_before_reporting() {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, content_digest,
    };
    let fixture = crate::command::workflow::workflow_decompose_reverify_tests::fixture();
    let tasks = fixture.project.join("tasks/PRD-X");
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    contract.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"command", "command":"test -f fixture.txt", "cwd":"project_root"})).unwrap();
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let digest = content_digest(&bytes);
    std::fs::write(&contract_path, bytes).unwrap();
    let lock_path = tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock["digest"] = digest.clone().into();
    std::fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        &fixture.project,
        &tasks,
        &digest,
        &["TASK-X-010", "TASK-X-020"],
    );
    let client = Arc::new(RecoveringJudge(AtomicUsize::new(0)));
    let _injection =
        super::super::acceptance::judge_client::inject(&fixture.project, client.clone());
    let state = fixture.store.load_state(&fixture.run_id).unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30000),
        checks(
            (&fixture.store, &fixture.run_id, state.generation, None),
            (
                &fixture.project,
                &fixture.project.join("prds/PRD-X.md"),
                &tasks,
            ),
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&Default::default()),
        ),
    )
    .await
    .expect("reverification must recover within the scripted judge windows")
    .unwrap();
    assert!(client.0.load(Ordering::SeqCst) >= 4);
    assert!(!result.operational_failure);
    assert_eq!(result.checks[0].verdict, "accepted");
    assert_eq!(
        result.checks[0].remediation_route,
        Some(acceptance::RemediationRoute::Task)
    );
}
