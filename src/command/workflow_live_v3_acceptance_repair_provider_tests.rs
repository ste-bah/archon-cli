use super::super::repair_tests::{record_baseline, run_fixture_with_mode};
use super::*;
use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};

struct OtherProvider;

#[async_trait::async_trait]
impl WorkflowLlmClient for OtherProvider {
    fn provider_id(&self) -> Option<String> {
        Some("provider-other".into())
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("provider mismatch must be detected before a request")
    }
}

#[tokio::test]
async fn provider_refusal_keeps_its_category_in_the_round_record() {
    let run = run_fixture_with_mode(
        &[("AC-X-001", "test -f missing", false)],
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
    );
    record_baseline(&run);
    let context = exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .unwrap();
    let contract = exec::load_contract(&context).unwrap().0;
    let base =
        crate::command::workflow_task_set::executability::Baseline::head_of(run.set.project.path())
            .unwrap()
            .commit;
    let (repair, _) = repair_unaccepted(Some(&OtherProvider), &context, &contract, Some(&base))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !repair.failure.contains(HOST_UNPROVEN),
        "{}",
        repair.failure
    );
    assert!(
        repair.failure.contains("provider-other"),
        "{}",
        repair.failure
    );
}
