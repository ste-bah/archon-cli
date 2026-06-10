use archon_workflow::{
    HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, ProviderFamily, ProviderTier,
    ProviderTierResolver, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    TemplateRegistry, WorkflowEvent, WorkflowExecutor, WorkflowPlanner, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore, classify_provider, stage::source_input_hash,
};

fn write_permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

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
    assert_eq!(
        rewound.stages.get("review").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        rewound.stages.get("synthesize").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        rewound.stages.get("quality").unwrap().status,
        StageStatus::Pending
    );
    let registry = TemplateRegistry::new(temp.path().join("templates"));
    let saved = registry.save("repo-audit", &spec).unwrap();
    assert!(saved.sanitized);
    assert!(registry.load("repo-audit").is_ok());
}

#[test]
fn resume_keeps_event_sequence_monotonic() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit this repo with subagents")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let first = executor.execute(run.clone()).unwrap();
    assert_eq!(first.failed, 0);

    LifecycleController::new(store.clone())
        .apply(&run.id, LifecycleAction::RestartStage("review".into()))
        .unwrap();
    let rewound = store.load_state(&run.id).unwrap();
    let resumed = executor.execute(rewound).unwrap();
    assert_eq!(resumed.failed, 0);

    let seqs = event_seqs(&store, &run.id);
    assert!(
        seqs.windows(2).all(|pair| pair[1] > pair[0]),
        "event seqs must increase strictly: {seqs:?}"
    );
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

#[tokio::test]
async fn live_executor_supplies_sources_and_dependency_artifacts() {
    struct ContextRunner {
        review_inputs: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait::async_trait]
    impl WorkflowStageRunner for ContextRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{"items":[{"path":"src/auth.rs","reason":"audit target"}]}"#,
                ));
            }
            if request.stage_id.starts_with("review-") {
                self.review_inputs
                    .lock()
                    .unwrap()
                    .push(request.input.clone());
                return Ok(StageRunOutput::markdown(
                    "reviewed concrete file: input == secret and seed + 1",
                ));
            }
            Ok(StageRunOutput::markdown("ok"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(
        temp.path().join("src/auth.rs"),
        "fn check(input: &str, secret: &str, seed: u64) { let _ = input == secret; let _ = seed + 1; }",
    )
    .unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec("Audit src/auth.rs")).unwrap();
    let runner = ContextRunner {
        review_inputs: std::sync::Mutex::new(Vec::new()),
    };
    let report = executor
        .execute_with_runner(run.clone(), &runner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let inputs = runner.review_inputs.lock().unwrap();
    let input = serde_json::to_string_pretty(&inputs[0]).unwrap();
    assert!(input.contains("input == secret"), "{input}");
    assert!(input.contains("seed + 1"), "{input}");
    assert!(input.contains("discover"), "{input}");
    assert!(input.contains("audit target"), "{input}");
}

#[tokio::test]
async fn blocked_fanout_output_fails_stage_instead_of_accepting() {
    struct BlockedRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for BlockedRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{"items":[{"path":"src/auth.rs","reason":"audit target"}]}"#,
                ));
            }
            Ok(StageRunOutput::markdown(
                "status: blocked\nfindings: []\nCannot audit because source evidence is missing.",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/auth.rs"), "fn check() {}").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec("Audit src/auth.rs")).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &BlockedRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Failed
    );
    let output_path = store
        .run_dir(&run.id)
        .join("agent-outputs")
        .join("review")
        .join("review-0.json");
    assert!(
        output_path.exists(),
        "blocked fanout output must be persisted for inspection"
    );
}

#[tokio::test]
async fn blocked_agent_output_is_persisted_for_inspection() {
    struct BlockedRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for BlockedRunner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                "status: blocked\nfindings: []\nCannot run tests because source evidence is missing.",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: blocked-agent-output
task: Run focused tests
stages:
  - id: focused_tests
    kind: agent
    task: Run focused tests and report exact commands.
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &BlockedRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let output_path = store
        .run_dir(&run.id)
        .join("agent-outputs")
        .join("focused_tests")
        .join("focused_tests.json");
    let output = std::fs::read_to_string(output_path).unwrap();
    assert!(output.contains("Cannot run tests because source evidence is missing"));
}

#[tokio::test]
async fn reject_verdict_fails_quality_gate() {
    struct RejectRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for RejectRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{"items":[{"path":"src/auth.rs","reason":"audit target"}]}"#,
                ));
            }
            Ok(StageRunOutput::markdown(
                "verdict: REJECT — DO NOT SIGN OFF\n\nBlocking findings remain.",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/auth.rs"), "fn check() {}").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec("Audit src/auth.rs")).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &RejectRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Failed
    );
}

#[tokio::test]
async fn failed_verification_output_fails_quality_gate_not_review_stage() {
    struct FailedFindingRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for FailedFindingRunner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                r#"{"unit_id":"VU-1","status":"failed","summary":"real blocker"}"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: failed-verification-gate
task: Check failed verification behavior.
stages:
  - id: review
    kind: agent
    agent: critic
  - id: quality
    kind: quality_gate
    depends_on: [review]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FailedFindingRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished.stages.get("quality").unwrap().status,
        StageStatus::Failed
    );
}

#[tokio::test]
async fn remediation_fanout_allows_empty_items_when_explicit() {
    struct EmptyInventoryRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "inventory" {
                return Ok(StageRunOutput::markdown(r#"{"items":[]}"#));
            }
            Ok(StageRunOutput::markdown("unexpected remediation item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-remediation
task: No-op clean remediation.
stages:
  - id: inventory
    kind: agent
    agent: critic
    outputs: [items]
  - id: remediate
    kind: fanout
    foreach: "${inventory.items}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyInventoryRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("remediate").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn provider_matrix_executes_code_and_research_workflows() {
    struct MatrixRunner {
        provider: &'static str,
    }

    #[async_trait::async_trait]
    impl WorkflowStageRunner for MatrixRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            // The `discover` stage is the structured fan-out items producer, so
            // it must emit a parseable `items:` document for the downstream
            // foreach fan-out to iterate over.
            let body = if request.stage_id == "discover" {
                format!(
                    r#"{{"items":[{{"provider":"{}","unit":"a"}},{{"provider":"{}","unit":"b"}}]}}"#,
                    self.provider, self.provider
                )
            } else {
                format!("{} handled {}", self.provider, request.stage_id)
            };
            Ok(StageRunOutput {
                body,
                extension: "md".into(),
                provider_id: Some(self.provider.into()),
                resolved_model: Some(format!("{}-test-model", self.provider)),
                tokens_in: 1,
                tokens_out: 1,
                cost_usd: 0.0,
            })
        }
    }

    for provider in [
        "anthropic",
        "openai-codex",
        "gemini",
        "deepseek",
        "ollama",
        "lm-studio",
    ] {
        for task in [
            "Audit this repo with subagents",
            "Research dynamic workflows",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = WorkflowStore::new(temp.path().join("workflows"));
            let spec = HeuristicWorkflowPlanner.plan(task).unwrap();
            let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
            let run = executor.start(spec).unwrap();
            let report = executor
                .execute_with_runner(run, &MatrixRunner { provider })
                .await
                .unwrap();
            assert_eq!(report.failed, 0, "{provider} failed {task}");
        }
    }
}

#[test]
fn template_save_rejects_embedded_secret_text() {
    let temp = tempfile::tempdir().unwrap();
    let mut spec = HeuristicWorkflowPlanner.plan("Research a topic").unwrap();
    spec.stages[0].input = serde_json::json!({
        "note": "Authorization: Bearer should-not-be-saved"
    });
    let err = TemplateRegistry::new(temp.path().join("templates"))
        .save("unsafe", &spec)
        .unwrap_err();
    assert!(err.to_string().contains("credential-like"));
}

#[test]
fn crash_after_artifact_write_resumes_without_duplicate_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit this repo with subagents")
        .unwrap();
    let discover = spec
        .stages
        .iter()
        .find(|stage| stage.id == "discover")
        .unwrap()
        .clone();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    store
        .write_artifact(
            &run.id,
            "discover",
            &source_input_hash(&discover),
            "md",
            b"artifact written before crash",
        )
        .unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    let discover_state = finished.stages.get("discover").unwrap();
    assert_eq!(discover_state.status, StageStatus::Accepted);
    assert_eq!(discover_state.artifacts.len(), 1);
}

fn event_seqs(store: &WorkflowStore, run_id: &str) -> Vec<u64> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<WorkflowEvent>(line).unwrap().seq)
        .collect()
}

fn audit_spec(task: &str) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: audit-test
task: {task}
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: review
    kind: fanout
    agent: workflow-reviewer
    foreach: ${{discover.items}}
    depends_on: [discover]
  - id: synthesize
    kind: reduce
    reducer: evidence_weighted_report
    depends_on: [review]
"#
    ))
    .unwrap()
}

#[tokio::test]
async fn declared_foreach_fanout_with_no_items_fails_fast() {
    // When a fan-out declares `foreach: ${discover.items}` but the producer
    // emits no parseable, non-empty `items:` structure, the runtime must fail
    // fast instead of collapsing to one synthetic item that the agent would
    // (correctly) reject as missing evidence.
    struct EmptyItemsRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyItemsRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                // No `items:` document at all — just prose.
                return Ok(StageRunOutput::markdown(
                    "Discovery complete. No structured items emitted.",
                ));
            }
            Ok(StageRunOutput::markdown("review body"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec("Audit with no items")).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyItemsRunner)
        .await
        .unwrap();
    assert_eq!(
        report.failed, 1,
        "fanout must fail when producer emits no items"
    );

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Failed
    );
}
