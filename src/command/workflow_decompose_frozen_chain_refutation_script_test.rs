use super::*;
use archon_workflow::WorkflowLlmClientFactory;
use std::sync::Arc;

#[path = "../../tests/support/workflow_decomposition_offline_server.rs"]
mod offline_provider;

struct RefuteThenOfflineProvider {
    reauthor: std::sync::Mutex<Option<Arc<dyn archon_workflow::WorkflowLlmClient>>>,
    provider: crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory,
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for RefuteThenOfflineProvider {
    async fn build_client(
        &self,
        request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn archon_workflow::WorkflowLlmClient>> {
        if let Some(client) = self.reauthor.lock().unwrap().take() {
            return Ok(client);
        }
        self.provider.build_client(request).await
    }
}

#[tokio::test]
async fn a_judge_refutation_runs_the_script_acceptance_author_and_freeze() {
    let fixture = super::super::frozen_with_prd(
        Some(names_live_root),
        include_str!("../../tests/fixtures/offline_decomposition/prd.md"),
    );
    let fixture_repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/offline_decomposition/repository");
    copy_tree(&fixture_repository, &fixture.project);
    with_skeleton(&fixture);
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);

    let provider = offline_provider::FakeProvider::start();
    let mut config = archon_core::config::ArchonConfig::default();
    config.api.base_url = Some(provider.base_url.clone());
    config.workflow.repository_root = Some(fixture.project.clone());
    let env_vars = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::from([(
        "ANTHROPIC_API_KEY".to_string(),
        "offline-test-key".to_string(),
    )]));
    let network_factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::new(
        &config, &env_vars,
    );
    let reauthor = Arc::new(
        crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
            |entry, _| {
                crate::command::workflow_task_set::reauthor::test_client::command_entry(
                    entry,
                    "test -f marker",
                )
            },
            |_, _| false,
        ),
    );
    let factory = RefuteThenOfflineProvider {
        reauthor: std::sync::Mutex::new(Some(reauthor)),
        provider: network_factory,
    };

    crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &config,
        &env_vars,
        &factory,
    )
    .await
    .unwrap_or_else(|error| panic!("the real decomposition script failed: {error:#}"));

    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let run_dir = fixture.store.run_dir(&run.id);
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(log.contains("acceptance=reauthor_refuted_routed"), "{log}");
    assert!(!log.contains("acceptance=reauthor_failed"), "{log}");
    let results = std::fs::read_dir(run_dir.join("v2/results"))
        .unwrap()
        .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
        .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap())
        .collect::<Vec<_>>();
    assert!(
        results.iter().any(|record| {
            record["call"]["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("acceptance-author-"))
                && record["status"] == "accepted"
        }),
        "the script's acceptance author call must be persisted: {results:?}"
    );
    assert!(
        results.iter().any(|record| {
            record["call"]["options"]["host_command"]["commandId"] == "freeze-acceptance"
                && record["status"] == "accepted"
        }),
        "the authored output must reach the acceptance freeze: {results:?}"
    );
    assert!(
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE).is_file(),
        "the acceptance freeze must publish its contract"
    );
    let requests = provider.requests.snapshot();
    assert!(
        requests.iter().any(|request| request.role == "acceptance"),
        "wire-level offline provider served no acceptance-author request: {requests:?}"
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}
