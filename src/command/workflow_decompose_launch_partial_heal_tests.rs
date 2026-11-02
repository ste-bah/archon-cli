use super::*;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct PartialJudge {
    inner: ScriptedAuthorJudge,
    failed: AtomicBool,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for PartialJudge {
    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }
    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }
    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.inner.run_agent(call).await
    }
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let prompt = messages[0]["content"].as_str().unwrap();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(prompt.rsplit_once("Checks: ").unwrap().1).unwrap();
        if !prompt.contains("\"baseline\":")
            && checks.len() == 1
            && checks[0]["id"] == "AC-X-002"
            && !self.failed.swap(true, Ordering::SeqCst)
        {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )));
        }
        self.inner
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await
    }
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.send_message_with_temperature(messages, system, tools, model, 0.0)
            .await
    }
}
struct Factory(Arc<PartialJudge>);
#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for Factory {
    async fn build_client(
        &self,
        _: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn workflow_decompose_launch_partial_publication_retries_only_pending_entries() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[
        ("AC-X-001", "test -f before-a", false),
        ("AC-X-002", "test -f before-b", false),
    ]);
    let client = Arc::new(PartialJudge {
        inner: ScriptedAuthorJudge::new(
            |entry, attempt| {
                if entry["id"] == "AC-X-002" && attempt == 1 {
                    "no entry".into()
                } else {
                    command_entry(entry, &format!("test -f marker-{attempt}"))
                }
            },
            |_, _| true,
        ),
        failed: AtomicBool::new(false),
    });
    let store = WorkflowStore::project(set.project.path());
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "repair".into(),
            task: "repair".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = archon_workflow::RunStatus::Running;
    store.save_state(&run).unwrap();
    let decision = LaunchAcceptance {
        reauthor: [
            ("AC-X-001".into(), "repair the check".into()),
            ("AC-X-002".into(), "repair the check".into()),
        ]
        .into(),
        ..Default::default()
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        repair(
            &Factory(client.clone()),
            &decision,
            (&store, &run.id),
            (set.project.path(), &set.prd, &set.tasks, set.project.path()),
            Default::default(),
        ),
    )
    .await
    .expect("partial repair must recover")
    .unwrap();
    assert!(
        client.failed.load(Ordering::SeqCst),
        "the transient outage must occur"
    );
    assert_eq!(result.1["pending_ids"], serde_json::json!([]));
    assert_eq!(
        result.1["published_ids"],
        serde_json::json!(["AC-X-001", "AC-X-002"])
    );
    let pin = set.pin();
    assert_eq!(pin.lineage.len(), 2);
    assert_eq!(
        pin.lineage[0].reauthored_ids,
        std::collections::BTreeSet::from(["AC-X-001".to_string()])
    );
    assert_eq!(
        pin.lineage[1].reauthored_ids,
        std::collections::BTreeSet::from(["AC-X-002".to_string()]),
        "a committed entry must never be published again during availability recovery"
    );
}
