//! Regression tests for finding O1: `Subtask::dependencies` used to be
//! honoured only in `ExecutionMode::Dag`.
//!
//! These drive the private mode runners directly, because `run_team`
//! synthesises dependencies only for `Pipeline` and `Dag` — the defect was
//! latent for `Parallel` and `Sequential` precisely because nothing in-tree
//! could reach it. Wave scheduling closes it structurally rather than relying
//! on callers never doing the wrong thing.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::config::OrchestratorConfig;
use super::events::{OrchestratorEvent, Subtask};
use super::{Orchestrator, SubtaskExecutor, topology};

/// Records `(subtask id, context received)` in completion order.
#[derive(Default)]
struct RecordingExecutor {
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl SubtaskExecutor for RecordingExecutor {
    async fn execute(&self, subtask: &Subtask, context: &str) -> anyhow::Result<String> {
        // Yield so a genuinely-concurrent scheduler would interleave and the
        // recorded order would stop being deterministic.
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        self.calls
            .lock()
            .expect("recording mutex")
            .push((subtask.id.clone(), context.to_string()));
        Ok(format!("result-of-{}", subtask.id))
    }
}

fn chain(ids: &[(&str, &[&str])]) -> Vec<Subtask> {
    ids.iter()
        .map(|(id, dependencies)| {
            let mut subtask =
                Subtask::new((*id).to_string(), format!("do {id}"), "coder".to_string());
            subtask.dependencies = dependencies.iter().map(|d| (*d).to_string()).collect();
            subtask
        })
        .collect()
}

/// The receiver is returned so the caller keeps it alive: every send site
/// ignores its result, but dropping the receiver would close the channel and
/// obscure which sends actually happened.
fn harness() -> (
    Orchestrator,
    Arc<RecordingExecutor>,
    mpsc::Sender<OrchestratorEvent>,
    mpsc::Receiver<OrchestratorEvent>,
) {
    let orchestrator = Orchestrator::new(OrchestratorConfig::default());
    let executor = Arc::new(RecordingExecutor::default());
    let (tx, rx) = mpsc::channel(128);
    (orchestrator, executor, tx, rx)
}

#[tokio::test]
async fn parallel_mode_respects_dependencies() {
    let (orchestrator, executor, tx, _events) = harness();
    // Deliberately supplied out of dependency order.
    let subtasks = chain(&[("C", &["B"]), ("A", &[]), ("B", &["A"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");
    assert_eq!(waves, vec![vec!["A"], vec!["B"], vec!["C"]]);

    orchestrator
        .run_parallel(subtasks, waves, executor.clone(), &tx)
        .await
        .expect("run");

    let calls = executor.calls.lock().expect("recording mutex").clone();
    let order: Vec<&str> = calls.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(order, vec!["A", "B", "C"]);
    // Dependency results reach the dependent as context.
    assert_eq!(calls[0].1, "");
    assert_eq!(calls[1].1, "result-of-A");
    assert_eq!(calls[2].1, "result-of-B");
}

#[tokio::test]
async fn parallel_mode_without_dependencies_is_one_wave() {
    // The shape `run_team` actually constructs for Parallel. Every task lands
    // in wave 0 with empty context, exactly as before wave scheduling.
    let (orchestrator, executor, tx, _events) = harness();
    let subtasks = chain(&[("A", &[]), ("B", &[]), ("C", &[])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");
    assert_eq!(waves.len(), 1);

    orchestrator
        .run_parallel(subtasks, waves, executor.clone(), &tx)
        .await
        .expect("run");

    let calls = executor.calls.lock().expect("recording mutex").clone();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|(_, context)| context.is_empty()));
}

#[tokio::test]
async fn sequential_mode_reorders_dependency_carrying_subtasks() {
    let (orchestrator, executor, tx, _events) = harness();
    let subtasks = chain(&[("C", &["B"]), ("A", &[]), ("B", &["A"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    orchestrator
        .run_sequential(subtasks, waves, executor.clone(), &tx)
        .await
        .expect("run");

    let calls = executor.calls.lock().expect("recording mutex").clone();
    let order: Vec<&str> = calls.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(order, vec!["A", "B", "C"]);
}

#[tokio::test]
async fn sequential_mode_threads_the_previous_result_when_dataflow_is_unknown() {
    // No dependencies means dataflow is *unknown*, not *none* — the previous
    // result keeps flowing forward, which is what Sequential always did.
    let (orchestrator, executor, tx, _events) = harness();
    let subtasks = chain(&[("A", &[]), ("B", &[]), ("C", &[])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    orchestrator
        .run_sequential(subtasks, waves, executor.clone(), &tx)
        .await
        .expect("run");

    let calls = executor.calls.lock().expect("recording mutex").clone();
    let order: Vec<&str> = calls.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(order, vec!["A", "B", "C"]);
    assert_eq!(calls[0].1, "");
    assert_eq!(calls[1].1, "result-of-A");
    assert_eq!(calls[2].1, "result-of-B");
}

#[tokio::test]
async fn pipeline_shaped_subtasks_keep_their_construction_order() {
    // `run_team` gives Pipeline a linear dependency chain, so waves are
    // singletons in construction order and nothing about execution changes.
    let subtasks = chain(&[
        ("task-0", &[]),
        ("task-1", &["task-0"]),
        ("task-2", &["task-1"]),
    ]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");
    assert_eq!(waves, vec![vec!["task-0"], vec!["task-1"], vec!["task-2"]]);
}

#[test]
fn lowering_is_lossy_in_exactly_the_documented_ways() {
    let mut subtask = Subtask::new("t1".into(), "describe".into(), "reviewer".into());
    subtask.dependencies = vec![];
    subtask.max_retries = 4;

    let graph = topology::lower_subtasks(&[subtask], "session-9");
    let node = graph.node("t1").expect("node present");

    assert_eq!(node.agent.as_deref(), Some("reviewer"));
    assert_eq!(node.role, archon_topology::NodeRole::Work);
    assert_eq!(node.permission, archon_topology::PermissionClass::Safe);
    // No dataflow and no write targets exist to recover — empty means unknown.
    assert!(!node.dataflow_is_known());
    assert!(!node.writes_are_known());
    assert!(node.fanout.is_none());

    assert_eq!(
        graph.origin,
        archon_topology::GraphOrigin::Team {
            session_id: "session-9".into()
        }
    );
    assert_eq!(graph.budget.max_agents, 1);
    // Retries are the only repetition a team performs.
    assert_eq!(graph.budget.max_rounds, 5);
}

#[test]
fn analyses_stay_silent_on_a_team_graph() {
    // A team lowering declares no writes and no permissions, so the two
    // analyses that depend on them must report nothing rather than guess.
    let subtasks = chain(&[("A", &[]), ("B", &[]), ("C", &["A"])]);
    let graph = topology::lower_subtasks(&subtasks, "session-1");

    assert!(graph.write_conflicts().expect("valid dag").is_empty());
    assert!(graph.ungated_irreversible().expect("valid dag").is_empty());
    // Structural analyses still work, which is the point of the lowering.
    assert_eq!(graph.critical_path().expect("valid dag").span(), 2);
    assert_eq!(
        graph.parallelism_profile().expect("valid dag").wave_widths,
        vec![2, 1]
    );
}

#[test]
fn duplicate_subtask_ids_are_rejected_rather_than_silently_shadowed() {
    let subtasks = chain(&[("A", &[]), ("A", &[])]);
    let error = topology::build_dag_waves(&subtasks).expect_err("duplicate id");
    assert!(error.to_string().contains("duplicate subtask id 'A'"));
}
