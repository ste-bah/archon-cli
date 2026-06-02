use archon_workflow::{
    HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, ProviderFamily, ProviderTier,
    ProviderTierResolver, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    TemplateRegistry, WorkflowExecutor, WorkflowPlanner, WorkflowPolicy, WorkflowStageRunner,
    WorkflowStore, classify_provider,
};

#[test]
fn provider_tiers_cover_supported_families() {
    for (input, family) in [
        ("anthropic", ProviderFamily::Anthropic),
        ("openai-codex", ProviderFamily::OpenAiCodex),
        ("openai", ProviderFamily::OpenAi),
        ("gemini", ProviderFamily::Gemini),
        ("deepseek", ProviderFamily::DeepSeek),
        ("ollama", ProviderFamily::Ollama),
        ("lm-studio", ProviderFamily::LmStudio),
    ] {
        assert_eq!(classify_provider(input).unwrap(), family);
    }
    let resolver = ProviderTierResolver::new("openai-codex", "gpt-5.5");
    let resolved = resolver.resolve(ProviderTier::Planner).unwrap();
    assert_eq!(resolved.provider_family, ProviderFamily::OpenAiCodex);
}

#[test]
fn planner_executor_lifecycle_and_template_work() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = HeuristicWorkflowPlanner;
    let spec = planner.plan("Audit this repo with subagents").unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec.clone()).unwrap();
    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    let controller = LifecycleController::new(store.clone());
    let rewound = controller
        .apply(&run.id, LifecycleAction::RestartStage("review".into()))
        .unwrap();
    assert_eq!(rewound.status, RunStatus::Running);
    let registry = TemplateRegistry::new(temp.path().join("templates"));
    let saved = registry.save("repo-audit", &spec).unwrap();
    assert!(saved.sanitized);
    assert!(registry.load("repo-audit").is_ok());
}

#[tokio::test]
async fn live_executor_routes_fanout_through_runner() {
    struct CaptureRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for CaptureRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(format!(
                "runner saw {:?} {}",
                request.stage_kind, request.stage_id
            )))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let mut spec = HeuristicWorkflowPlanner
        .plan("Audit this repo with subagents")
        .unwrap();
    let review = spec
        .stages
        .iter_mut()
        .find(|stage| stage.id == "review")
        .unwrap();
    review.input = serde_json::json!({"items": [{"module": "a"}, {"module": "b"}]});
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    let report = executor
        .execute_with_runner(run, &CaptureRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run_id).unwrap();
    let review = finished.stages.get("review").unwrap();
    assert_eq!(review.status, StageStatus::Accepted);
    assert_eq!(review.artifacts.len(), 2);
    let artifacts = review
        .artifacts
        .iter()
        .map(|artifact| {
            std::fs::read_to_string(store.run_dir(&run_id).join(&artifact.path)).unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        artifacts
            .iter()
            .any(|body| body.contains("Fanout review-0"))
    );
    assert!(
        artifacts
            .iter()
            .any(|body| body.contains("Fanout review-1"))
    );
}
