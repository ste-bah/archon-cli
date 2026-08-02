//! Regression tests for findings O2 and O4 — the agent pool.
//!
//! O4: `run_dag_waves` never touched `AgentPool` at all while `run_parallel`
//! did, so `ExecutionMode::Dag` had no cap of any kind — a wide wave spawned
//! every task in it at once regardless of `max_concurrent`.
//!
//! O2: the pool released its slot on completion and so imposed no lifetime
//! total, leaving a team free to start an unbounded number of agents.
//!
//! The lifetime budget is the sharper probe for both, because unlike the
//! concurrency cap it cannot be satisfied by waiting: a runner that does not
//! consult the pool passes a concurrency test by accident whenever the work is
//! fast enough.

use std::sync::Arc;

use tokio::sync::mpsc;

use super::config::OrchestratorConfig;
use super::events::OrchestratorEvent;
use super::wave_scheduling_tests::{RecordingExecutor, chain};
use super::{Orchestrator, topology};

type Harness = (
    Orchestrator,
    Arc<RecordingExecutor>,
    mpsc::Sender<OrchestratorEvent>,
    mpsc::Receiver<OrchestratorEvent>,
);

/// The receiver is returned so the caller keeps it alive; dropping it would
/// close the channel and every send site ignores its result.
fn harness_with(max_agents: u32) -> Harness {
    let orchestrator = Orchestrator::new(OrchestratorConfig {
        max_agents,
        ..OrchestratorConfig::default()
    });
    let (tx, rx) = mpsc::channel(128);
    (orchestrator, Arc::new(RecordingExecutor::default()), tx, rx)
}

#[tokio::test]
async fn dag_waves_go_through_the_agent_pool() {
    let (orchestrator, executor, tx, _events) = harness_with(2);
    let subtasks = chain(&[("A", &[]), ("B", &["A"]), ("C", &["B"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    let error = orchestrator
        .run_dag_waves(subtasks, waves, executor.clone(), &tx)
        .await
        .expect_err("the third agent exceeds a lifetime budget of two");

    assert!(
        error.to_string().contains("lifetime budget exhausted"),
        "{error}"
    );
    assert_eq!(executor.recorded().len(), 2);
}

#[tokio::test]
async fn sequential_mode_also_spends_lifetime_budget() {
    // One agent at a time never strains a concurrency cap, so without lifetime
    // accounting `max_agents` would be a property of the execution mode rather
    // than of the team.
    let (orchestrator, executor, tx, _events) = harness_with(2);
    let subtasks = chain(&[("A", &[]), ("B", &["A"]), ("C", &["B"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    let error = orchestrator
        .run_sequential(subtasks, waves, executor.clone(), &tx)
        .await
        .expect_err("the third agent exceeds a lifetime budget of two");

    assert!(
        error.to_string().contains("lifetime budget exhausted"),
        "{error}"
    );
    assert_eq!(executor.recorded().len(), 2);
}

#[tokio::test]
async fn parallel_mode_spends_lifetime_budget() {
    let (orchestrator, executor, tx, _events) = harness_with(2);
    let subtasks = chain(&[("A", &[]), ("B", &["A"]), ("C", &["B"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    let error = orchestrator
        .run_parallel(subtasks, waves, executor.clone(), &tx)
        .await
        .expect_err("the third agent exceeds a lifetime budget of two");

    assert!(
        error.to_string().contains("lifetime budget exhausted"),
        "{error}"
    );
    assert_eq!(executor.recorded().len(), 2);
}

/// The near-miss: a run that fits the budget exactly still completes, in every
/// mode. Without this the three tests above would pass for a pool that refused
/// everything.
#[tokio::test]
async fn a_run_inside_the_lifetime_budget_completes_in_every_mode() {
    let subtasks = chain(&[("A", &[]), ("B", &["A"]), ("C", &["B"])]);
    let (_, waves) = topology::plan(&subtasks, "team", 4).expect("valid dag");

    for mode in ["dag", "parallel", "sequential"] {
        let (orchestrator, executor, tx, _events) = harness_with(3);
        let subtasks = subtasks.clone();
        let waves = waves.clone();
        let result = match mode {
            "dag" => {
                orchestrator
                    .run_dag_waves(subtasks, waves, executor.clone(), &tx)
                    .await
            }
            "parallel" => {
                orchestrator
                    .run_parallel(subtasks, waves, executor.clone(), &tx)
                    .await
            }
            _ => {
                orchestrator
                    .run_sequential(subtasks, waves, executor.clone(), &tx)
                    .await
            }
        };

        assert!(result.is_ok(), "{mode}: {result:?}");
        assert_eq!(executor.recorded().len(), 3, "{mode}");
    }
}
