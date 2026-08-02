pub mod config;
pub mod events;
#[path = "orchestrator_executor.rs"]
mod executor;
pub mod pool;
mod scheduling;
pub mod topology;
pub use executor::RealSubtaskExecutor;

#[cfg(test)]
#[path = "orchestrator_compaction_tests.rs"]
mod compaction_tests;

// Ordinary `mod` — resolves to orchestrator/wave_scheduling_tests.rs. No
// `#[path]` splicing; see docs/architecture/topology-engineering.md, CL2.
#[cfg(test)]
mod wave_scheduling_tests;

use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use config::{ExecutionMode, OrchestratorConfig, TeamConfig};
use events::{OrchestratorEvent, Subtask, SubtaskStatus};
use pool::AgentPool;
use scheduling::{dependency_context, flatten_waves, retry_execute};

/// Trait for executing a single subtask. Tests supply mocks; production wires the agent loop.
#[async_trait::async_trait]
pub trait SubtaskExecutor: Send + Sync {
    async fn execute(&self, subtask: &Subtask, context: &str) -> anyhow::Result<String>;
}

/// Default executor used by CLI: logs the subtask and returns a structured placeholder.
/// Full LLM agent wiring is a Phase 6 concern. The orchestration infrastructure is complete.
pub struct LoggingExecutor;

#[async_trait::async_trait]
impl SubtaskExecutor for LoggingExecutor {
    async fn execute(&self, subtask: &Subtask, _context: &str) -> anyhow::Result<String> {
        tracing::info!(
            "orchestrator: executing subtask {} ({}) via agent {}",
            subtask.id,
            subtask.description,
            subtask.agent_type
        );
        Ok(format!(
            "[{}:{}] {}",
            subtask.agent_type, subtask.id, subtask.description
        ))
    }
}

pub struct Orchestrator {
    config: OrchestratorConfig,
    cancelled: Arc<Mutex<bool>>,
}

impl Orchestrator {
    pub fn new(config: OrchestratorConfig) -> Self {
        Self {
            config,
            cancelled: Arc::new(Mutex::new(false)),
        }
    }

    /// A pool carrying both ceilings this orchestrator is configured with.
    ///
    /// One constructor for every runner, so the O4 defect — one runner using
    /// the pool and another not — cannot recur by omission at a second call
    /// site.
    fn agent_pool(&self) -> AgentPool {
        AgentPool::with_lifetime_cap(self.config.max_concurrent, self.config.max_agents)
    }

    pub async fn run_team(
        &self,
        team: TeamConfig,
        goal: String,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        tracing::info!(
            "orchestrator: starting team '{}' mode={:?} goal={}",
            team.name,
            team.mode,
            goal
        );

        // Build initial plan: one subtask per agent type in the team
        let subtasks: Vec<Subtask> = team
            .agents
            .iter()
            .enumerate()
            .map(|(i, agent_type)| {
                let mut t = Subtask::new(
                    format!("task-{i}"),
                    format!("{goal} [assigned to {agent_type}]"),
                    agent_type.clone(),
                );
                // Pipeline/DAG: each task depends on the previous one
                if matches!(team.mode, ExecutionMode::Pipeline | ExecutionMode::Dag) && i > 0 {
                    t.dependencies.push(format!("task-{}", i - 1));
                }
                t
            })
            .collect();

        let _ = event_tx
            .send(OrchestratorEvent::TaskDecomposed {
                subtasks: subtasks.clone(),
            })
            .await;

        // Every mode schedules against the same topology IR. Dependencies used
        // to be honoured only in `Dag` (finding O1); now the graph decides
        // *ordering* in all four modes and the mode decides only concurrency
        // within a wave and what happens on failure. For a decomposition that
        // declares no dependencies this collapses to a single wave, which is
        // exactly the old behaviour.
        let (graph, waves) = topology::plan(&subtasks, &team.name, self.config.max_concurrent)?;
        if let Ok(path) = graph.critical_path() {
            tracing::debug!(
                "orchestrator: team '{}' has {} wave(s), span {}",
                team.name,
                waves.len(),
                path.span()
            );
        }

        let result = match team.mode {
            ExecutionMode::Sequential | ExecutionMode::Pipeline => {
                self.run_sequential(subtasks, waves, executor, &event_tx)
                    .await?
            }
            ExecutionMode::Parallel => {
                self.run_parallel(subtasks, waves, executor, &event_tx)
                    .await?
            }
            ExecutionMode::Dag => {
                self.run_dag_waves(subtasks, waves, executor, &event_tx)
                    .await?
            }
        };

        let _ = event_tx
            .send(OrchestratorEvent::TeamComplete {
                result: result.clone(),
            })
            .await;

        Ok(result)
    }

    /// One task at a time, in dependency order, stopping at the first failure.
    ///
    /// Ordering comes from the waves rather than from the vector, so a
    /// dependency-carrying decomposition is now executed in a valid order
    /// instead of whatever order it was constructed in. `Sequential` declares
    /// no dependencies and `Pipeline` declares a linear chain, so for both the
    /// flattened wave order equals the original vector order and this is
    /// behaviour-identical to the previous implementation.
    async fn run_sequential(
        &self,
        subtasks: Vec<Subtask>,
        waves: Vec<Vec<String>>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        // One at a time, so concurrency is never in question — but the lifetime
        // budget still applies, and applying it here rather than only in the
        // concurrent runners is what makes `max_agents` a property of the team
        // rather than of the execution mode it happens to run under.
        let pool = self.agent_pool();
        let mut ordered = flatten_waves(&subtasks, &waves);
        let mut completed: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut context = String::new();
        let mut results = Vec::new();

        for subtask in &mut ordered {
            if *self.cancelled.lock().await {
                // agent-event-tx-lint: ignore — channel holds OrchestratorEvent, not AgentEvent
                let _ = event_tx.send(OrchestratorEvent::TeamCancelled).await;
                anyhow::bail!("team cancelled");
            }

            // No declared dependencies means dataflow is *unknown*, not
            // *none* — so keep threading the previous result forward rather
            // than handing the task an empty context it never asked for. When
            // dependencies are declared, they are the dataflow.
            if !subtask.dependencies.is_empty() {
                context = subtask
                    .dependencies
                    .iter()
                    .filter_map(|dependency| completed.get(dependency))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
            }

            let agent_id = format!("agent-{}", subtask.id);
            pool.acquire(
                agent_id.clone(),
                subtask.id.clone(),
                subtask.agent_type.clone(),
            )
            .await?;
            let _ = event_tx
                .send(OrchestratorEvent::AgentSpawned {
                    agent_id: agent_id.clone(),
                    agent_type: subtask.agent_type.clone(),
                    subtask_id: subtask.id.clone(),
                })
                .await;

            subtask.status = SubtaskStatus::Running;

            let attempt = self
                .execute_with_retry(subtask, &context, executor.as_ref())
                .await;
            // Concurrency slot back immediately; the lifetime unit stays spent.
            pool.release(&agent_id).await;

            match attempt {
                Ok(result) => {
                    let _ = event_tx
                        .send(OrchestratorEvent::AgentComplete {
                            agent_id: agent_id.clone(),
                            subtask_id: subtask.id.clone(),
                            result: result.clone(),
                        })
                        .await;
                    completed.insert(subtask.id.clone(), result.clone());
                    context = result.clone();
                    results.push(result);
                    subtask.status = SubtaskStatus::Complete {
                        result: context.clone(),
                    };
                }
                Err(e) => {
                    let _ = event_tx
                        .send(OrchestratorEvent::AgentFailed {
                            agent_id,
                            subtask_id: subtask.id.clone(),
                            error: e.to_string(),
                            will_retry: false,
                        })
                        .await;
                    subtask.status = SubtaskStatus::Failed {
                        error: e.to_string(),
                    };
                    return Err(e);
                }
            }
        }

        Ok(results.join("\n---\n"))
    }

    /// Maximum concurrency inside each wave, bounded by the agent pool,
    /// joining between waves. Failures are recorded, not propagated.
    ///
    /// This is the O1 fix: `run_parallel` previously ignored
    /// `Subtask::dependencies` entirely and spawned every task at once, so a
    /// dependency-carrying decomposition handed to `ExecutionMode::Parallel`
    /// ran its dependents before their dependencies. A decomposition with no
    /// dependencies — which is what `Parallel` constructs today — lowers to a
    /// single wave, so that path is unchanged.
    async fn run_parallel(
        &self,
        subtasks: Vec<Subtask>,
        waves: Vec<Vec<String>>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        let pool = self.agent_pool();
        let mut completed: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut results = Vec::new();

        for wave in &waves {
            let mut handles = Vec::new();

            for subtask in flatten_waves(&subtasks, std::slice::from_ref(wave)) {
                // The lifetime budget is not something waiting can clear, so
                // stop polling once it is exhausted and let `acquire` report it.
                while !pool.can_spawn().await && !pool.lifetime_exhausted().await {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }

                // Empty for a dependency-free task, which is the same context
                // the previous implementation always passed.
                let context = dependency_context(&subtask, &completed);

                let agent_id = format!("agent-{}", subtask.id);
                pool.acquire(
                    agent_id.clone(),
                    subtask.id.clone(),
                    subtask.agent_type.clone(),
                )
                .await?;

                let _ = event_tx
                    .send(OrchestratorEvent::AgentSpawned {
                        agent_id: agent_id.clone(),
                        agent_type: subtask.agent_type.clone(),
                        subtask_id: subtask.id.clone(),
                    })
                    .await;

                let exec = executor.clone();
                let pool_clone = pool.clone();
                let tx = event_tx.clone();
                let max_retries = self.config.max_retries;

                handles.push(tokio::spawn(async move {
                    let result =
                        retry_execute(&subtask, &context, exec.as_ref(), max_retries).await;
                    pool_clone.release(&agent_id).await;
                    match result {
                        Ok(r) => {
                            let _ = tx
                                .send(OrchestratorEvent::AgentComplete {
                                    agent_id,
                                    subtask_id: subtask.id.clone(),
                                    result: r.clone(),
                                })
                                .await;
                            Ok((subtask.id, r))
                        }
                        Err(e) => {
                            let _ = tx
                                .send(OrchestratorEvent::AgentFailed {
                                    agent_id,
                                    subtask_id: subtask.id,
                                    error: e.to_string(),
                                    will_retry: false,
                                })
                                .await;
                            Err(e.to_string())
                        }
                    }
                }));
            }

            for handle in handles {
                match handle.await {
                    Ok(Ok((id, r))) => {
                        results.push(r.clone());
                        completed.insert(id, r);
                    }
                    Ok(Err(e)) => results.push(format!("[FAILED: {e}]")),
                    Err(e) => results.push(format!("[PANIC: {e}]")),
                }
            }
        }

        Ok(results.join("\n---\n"))
    }

    /// Maximum concurrency inside each wave, bounded by the agent pool,
    /// joining between waves. Failures are recorded, not propagated.
    ///
    /// This is the O4 fix. `run_dag_waves` never touched `AgentPool` at all
    /// while `run_parallel` did, so `ExecutionMode::Dag` — the *only* mode that
    /// honoured dependencies before milestone 1 — had no concurrency cap
    /// whatsoever, not merely no lifetime total. A wide wave spawned every task
    /// in it at once regardless of `max_concurrent`.
    async fn run_dag_waves(
        &self,
        subtasks: Vec<Subtask>,
        waves: Vec<Vec<String>>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        let pool = self.agent_pool();
        let mut all_results: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut wave_results = Vec::new();

        for wave in waves {
            let wave_tasks: Vec<Subtask> = wave
                .iter()
                .filter_map(|id| subtasks.iter().find(|t| &t.id == id).cloned())
                .collect();

            let mut handles = Vec::new();
            for subtask in wave_tasks {
                let context = dependency_context(&subtask, &all_results);

                let agent_id = format!("agent-{}", subtask.id);
                // Wait for a concurrency slot, but never for the lifetime
                // budget — that one no amount of waiting clears, so `acquire`
                // is left to fail and the error propagates.
                while !pool.can_spawn().await && !pool.lifetime_exhausted().await {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
                pool.acquire(
                    agent_id.clone(),
                    subtask.id.clone(),
                    subtask.agent_type.clone(),
                )
                .await?;
                let _ = event_tx
                    .send(OrchestratorEvent::AgentSpawned {
                        agent_id: agent_id.clone(),
                        agent_type: subtask.agent_type.clone(),
                        subtask_id: subtask.id.clone(),
                    })
                    .await;

                let exec = executor.clone();
                let tx = event_tx.clone();
                let pool_clone = pool.clone();
                let max_retries = self.config.max_retries;

                handles.push(tokio::spawn(async move {
                    let result =
                        retry_execute(&subtask, &context, exec.as_ref(), max_retries).await;
                    pool_clone.release(&agent_id).await;
                    match result {
                        Ok(r) => {
                            let _ = tx
                                .send(OrchestratorEvent::AgentComplete {
                                    agent_id,
                                    subtask_id: subtask.id.clone(),
                                    result: r.clone(),
                                })
                                .await;
                            Ok((subtask.id, r))
                        }
                        Err(e) => {
                            let _ = tx
                                .send(OrchestratorEvent::AgentFailed {
                                    agent_id,
                                    subtask_id: subtask.id.clone(),
                                    error: e.to_string(),
                                    will_retry: false,
                                })
                                .await;
                            Err(e.to_string())
                        }
                    }
                }));
            }

            for handle in handles {
                match handle.await {
                    Ok(Ok((id, r))) => {
                        wave_results.push(r.clone());
                        all_results.insert(id, r);
                    }
                    Ok(Err(e)) => wave_results.push(format!("[FAILED: {e}]")),
                    Err(e) => wave_results.push(format!("[PANIC: {e}]")),
                }
            }
        }

        Ok(wave_results.join("\n---\n"))
    }

    async fn execute_with_retry(
        &self,
        subtask: &Subtask,
        context: &str,
        executor: &dyn SubtaskExecutor,
    ) -> anyhow::Result<String> {
        retry_execute(subtask, context, executor, self.config.max_retries).await
    }
}
