//! Phase A regression tests for PRD-009 audited gaps:
//! - Per-item `restart-agent` rewinds exactly one fan-out item (AC-US3-02,
//!   EC-DWF-18, TC-DWF-015).
//! - Whole-stage `restart-stage` remains distinct and rewinds dependents.
//! - Local-provider fan-out agent cap (OQ-DWF-003 / EC-DWF-21).

use archon_workflow::{
    LifecycleAction, LifecycleController, ProviderTier, RunStatus, StageRunOutput, StageRunRequest,
    StageStatus, WorkflowConfig, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

struct FanoutRunner;

impl archon_workflow::WriteBoundaryProbe for FanoutRunner {}
#[async_trait::async_trait]
impl WorkflowStageRunner for FanoutRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "discover" {
            return Ok(StageRunOutput::markdown(
                r#"{"items":[{"path":"a.rs"},{"path":"b.rs"},{"path":"c.rs"}]}"#,
            ));
        }
        Ok(StageRunOutput::markdown(format!(
            "reviewed {}",
            request.stage_id
        )))
    }
}

fn fanout_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: phase-a-test
task: per-item restart coverage
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: review
    kind: fanout
    agent: workflow-reviewer
    foreach: ${discover.items}
    depends_on: [discover]
  - id: synthesize
    kind: reduce
    reducer: evidence_weighted_report
    depends_on: [review]
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn restart_item_rewinds_exactly_one_item_and_preserves_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(fanout_spec()).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &FanoutRunner)
        .await
        .unwrap();

    let before = store.load_state(&run_id).unwrap();
    let item_ids: Vec<String> = before
        .items
        .values()
        .filter(|item| item.stage_id == "review")
        .map(|item| item.id.clone())
        .collect();
    assert_eq!(
        item_ids.len(),
        3,
        "three review items expected: {item_ids:?}"
    );
    let target = item_ids[1].clone();

    let rewound = LifecycleController::new(store.clone())
        .apply(
            &run_id,
            LifecycleAction::RestartItem {
                stage_id: "review".into(),
                item_id: target.clone(),
            },
        )
        .unwrap();

    assert_eq!(rewound.status, RunStatus::Running);
    assert!(!rewound.items.contains_key(&target), "target item removed");
    let surviving: Vec<&String> = rewound
        .items
        .values()
        .filter(|item| item.stage_id == "review")
        .map(|item| &item.id)
        .collect();
    assert_eq!(surviving.len(), 2, "two siblings preserved: {surviving:?}");
    assert_eq!(
        rewound.stages.get("review").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        rewound.stages.get("synthesize").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        rewound.stages.get("discover").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn restart_item_rejects_unknown_or_mismatched_item() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(fanout_spec()).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &FanoutRunner)
        .await
        .unwrap();

    let err = LifecycleController::new(store.clone())
        .apply(
            &run_id,
            LifecycleAction::RestartItem {
                stage_id: "review".into(),
                item_id: "review-999".into(),
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("unknown item"), "{err}");
}

#[tokio::test]
async fn restart_stage_rewinds_whole_stage_and_dependents() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(fanout_spec()).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &FanoutRunner)
        .await
        .unwrap();

    let rewound = LifecycleController::new(store.clone())
        .apply(&run_id, LifecycleAction::RestartStage("review".into()))
        .unwrap();

    assert!(
        rewound.items.values().all(|item| item.stage_id != "review"),
        "all review items cleared by whole-stage restart"
    );
    assert_eq!(
        rewound.stages.get("review").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        rewound.stages.get("synthesize").unwrap().status,
        StageStatus::Pending
    );
}

/// EC-DWF-21: a `local` provider-tier fan-out stage is capped by
/// `local_provider_max_agents` even when `max_agents` is large.
#[tokio::test]
async fn local_provider_fanout_is_capped() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let policy = WorkflowPolicy::from_config(&WorkflowConfig {
        local_provider_max_agents: 2,
        ..WorkflowConfig::default()
    });
    let executor = WorkflowExecutor::new(store.clone(), policy);
    let mut spec = fanout_spec();
    spec.max_agents = 100;
    let review = spec
        .stages
        .iter_mut()
        .find(|stage| stage.id == "review")
        .unwrap();
    review.provider_tier = Some(ProviderTier::Local);
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    // The cap denies the over-wide fan-out: the stage is marked failed (not
    // accepted), surfaced through the execution report rather than a hard error.
    let report = executor
        .execute_with_runner(run, &FanoutRunner)
        .await
        .unwrap();
    assert_eq!(
        report.failed, 1,
        "local cap should fail the 3 > 2 fan-out stage"
    );

    let finished = store.load_state(&run_id).unwrap();
    let review = finished.stages.get("review").unwrap();
    assert_eq!(review.status, StageStatus::Failed);
    assert!(
        review
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("max_agents"),
        "failure reason should cite the agent cap: {:?}",
        review.error
    );
}

/// A non-local tier with the same width is *not* capped by the local limit.
#[tokio::test]
async fn non_local_provider_fanout_ignores_local_cap() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let policy = WorkflowPolicy::from_config(&WorkflowConfig {
        local_provider_max_agents: 2,
        ..WorkflowConfig::default()
    });
    let executor = WorkflowExecutor::new(store.clone(), policy);
    let mut spec = fanout_spec();
    spec.max_agents = 100;
    let review = spec
        .stages
        .iter_mut()
        .find(|stage| stage.id == "review")
        .unwrap();
    review.provider_tier = Some(ProviderTier::Researcher);
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    let report = executor
        .execute_with_runner(run, &FanoutRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run_id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted
    );
}
