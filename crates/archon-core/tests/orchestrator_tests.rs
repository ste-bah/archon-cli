use archon_core::orchestrator::{
    config::{ExecutionMode, OrchestratorConfig, TeamConfig},
    dag::build_dag_waves,
    events::{OrchestratorEvent, Subtask, SubtaskStatus},
    planner::parse_plan,
    pool::AgentPool,
};

// 1. OrchestratorConfig::default() has max_concurrent=4, timeout_secs=300
#[test]
fn orchestrator_config_default_values() {
    let cfg = OrchestratorConfig::default();
    assert_eq!(cfg.max_concurrent, 4);
    assert_eq!(cfg.timeout_secs, 300);
    assert_eq!(cfg.max_retries, 2);
}

// 2. TeamConfig parses from valid TOML with name, coordinator, agents, mode
#[test]
fn team_config_parses_from_toml() {
    let toml_str = r#"
name = "dev-team"
coordinator = "planner"
agents = ["coder", "reviewer"]
mode = "parallel"
"#;
    let cfg: TeamConfig = toml::from_str(toml_str).expect("should parse TeamConfig");
    assert_eq!(cfg.name, "dev-team");
    assert_eq!(cfg.coordinator, "planner");
    assert_eq!(cfg.agents, vec!["coder", "reviewer"]);
    assert_eq!(cfg.mode, ExecutionMode::Parallel);
}

// 3. ExecutionMode variants serialize to/from snake_case strings
#[test]
fn execution_mode_serde_snake_case() {
    let modes = vec![
        (ExecutionMode::Sequential, "\"sequential\""),
        (ExecutionMode::Parallel, "\"parallel\""),
        (ExecutionMode::Pipeline, "\"pipeline\""),
        (ExecutionMode::Dag, "\"dag\""),
    ];
    for (mode, expected_json) in modes {
        let serialized = serde_json::to_string(&mode).expect("serialize");
        assert_eq!(serialized, expected_json);
        let deserialized: ExecutionMode = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized, mode);
    }
}

// 4. New Subtask has SubtaskStatus::Pending
#[test]
fn subtask_new_has_pending_status() {
    let t = Subtask::new("id-1".into(), "do something".into(), "coder".into());
    assert_eq!(t.status, SubtaskStatus::Pending);
}

// 5. Subtask status can transition to Running
#[test]
fn subtask_status_transition_to_running() {
    let mut t = Subtask::new("id-2".into(), "desc".into(), "coder".into());
    t.status = SubtaskStatus::Running;
    assert_eq!(t.status, SubtaskStatus::Running);
}

// 6. Subtask status can transition to Complete
#[test]
fn subtask_status_transition_to_complete() {
    let mut t = Subtask::new("id-3".into(), "desc".into(), "coder".into());
    t.status = SubtaskStatus::Complete {
        result: "done".into(),
    };
    assert!(matches!(t.status, SubtaskStatus::Complete { .. }));
}

// 7. Subtask status can transition to Failed
#[test]
fn subtask_status_transition_to_failed() {
    let mut t = Subtask::new("id-4".into(), "desc".into(), "coder".into());
    t.status = SubtaskStatus::Failed {
        error: "oops".into(),
    };
    assert!(matches!(t.status, SubtaskStatus::Failed { .. }));
}

// 8. OrchestratorEvent::TaskDecomposed can be created with a vec of subtasks
#[test]
fn orchestrator_event_task_decomposed_created() {
    let subtasks = vec![
        Subtask::new("t1".into(), "first task".into(), "coder".into()),
        Subtask::new("t2".into(), "second task".into(), "reviewer".into()),
    ];
    let event = OrchestratorEvent::TaskDecomposed {
        subtasks: subtasks.clone(),
    };
    match event {
        OrchestratorEvent::TaskDecomposed { subtasks: ref s } => {
            assert_eq!(s.len(), 2);
        }
        _ => panic!("wrong variant"),
    }
    // suppress unused warning
    let _ = subtasks;
}

// 9. OrchestratorEvent::TeamComplete can be created
#[test]
fn orchestrator_event_team_complete_created() {
    let event = OrchestratorEvent::TeamComplete {
        result: "all done".into(),
    };
    match event {
        OrchestratorEvent::TeamComplete { result } => {
            assert_eq!(result, "all done");
        }
        _ => panic!("wrong variant"),
    }
}

// 10. build_dag_waves: linear chain [A→B→C] gives waves [[A],[B],[C]]
#[test]
fn dag_linear_chain_gives_sequential_waves() {
    let tasks = vec![
        Subtask::new("A".into(), "a".into(), "coder".into()),
        {
            let mut t = Subtask::new("B".into(), "b".into(), "coder".into());
            t.dependencies = vec!["A".into()];
            t
        },
        {
            let mut t = Subtask::new("C".into(), "c".into(), "coder".into());
            t.dependencies = vec!["B".into()];
            t
        },
    ];
    let waves = build_dag_waves(&tasks).expect("no cycle");
    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0], vec!["A"]);
    assert_eq!(waves[1], vec!["B"]);
    assert_eq!(waves[2], vec!["C"]);
}

// 11. build_dag_waves: no dependencies gives all in one wave [[A,B,C]]
#[test]
fn dag_no_deps_gives_single_wave() {
    let tasks = vec![
        Subtask::new("A".into(), "a".into(), "coder".into()),
        Subtask::new("B".into(), "b".into(), "coder".into()),
        Subtask::new("C".into(), "c".into(), "coder".into()),
    ];
    let waves = build_dag_waves(&tasks).expect("no cycle");
    assert_eq!(waves.len(), 1);
    let mut wave = waves[0].clone();
    wave.sort();
    assert_eq!(wave, vec!["A", "B", "C"]);
}

// 12. build_dag_waves: cycle detection returns Err
#[test]
fn dag_cycle_returns_error() {
    let tasks = vec![
        {
            let mut t = Subtask::new("A".into(), "a".into(), "coder".into());
            t.dependencies = vec!["B".into()];
            t
        },
        {
            let mut t = Subtask::new("B".into(), "b".into(), "coder".into());
            t.dependencies = vec!["A".into()];
            t
        },
    ];
    let result = build_dag_waves(&tasks);
    assert!(result.is_err(), "cycle should be detected");
}

// 13. AgentPool::new(4): can_spawn() returns true initially, false when full
#[tokio::test]
async fn agent_pool_capacity_enforced() {
    let pool = AgentPool::new(2);
    assert!(pool.can_spawn().await);

    pool.acquire("a1".into(), "t1".into(), "coder".into())
        .await
        .expect("acquire 1");
    assert!(pool.can_spawn().await);

    pool.acquire("a2".into(), "t2".into(), "coder".into())
        .await
        .expect("acquire 2");
    assert!(!pool.can_spawn().await, "pool should be full at capacity 2");

    // Acquiring when full should return an error
    assert!(
        pool.acquire("a3".into(), "t3".into(), "coder".into())
            .await
            .is_err()
    );

    pool.release("a1").await;
    assert!(
        pool.can_spawn().await,
        "should have capacity again after release"
    );
}

// 14. parse_plan: parses valid coordinator JSON into subtasks
#[test]
fn parse_plan_valid_json() {
    let coordinator_output = r#"
Here is the plan I have prepared:
{
  "subtasks": [
    {"id": "1", "description": "write tests", "agent_type": "tester", "dependencies": []},
    {"id": "2", "description": "implement feature", "agent_type": "coder", "dependencies": ["1"]}
  ]
}
"#;
    let subtasks = parse_plan(coordinator_output).expect("should parse plan");
    assert_eq!(subtasks.len(), 2);
    assert_eq!(subtasks[0].id, "1");
    assert_eq!(subtasks[0].agent_type, "tester");
    assert_eq!(subtasks[1].id, "2");
    assert_eq!(subtasks[1].dependencies, vec!["1"]);
}
