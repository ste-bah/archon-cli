pub mod config;
pub mod dag;
pub mod events;
pub mod planner;
pub mod pool;

use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use config::{ExecutionMode, OrchestratorConfig, TeamConfig};
use events::{OrchestratorEvent, Subtask, SubtaskStatus};
use pool::AgentPool;

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

    pub async fn cancel(&self) {
        *self.cancelled.lock().await = true;
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

        let result = match team.mode {
            ExecutionMode::Sequential | ExecutionMode::Pipeline => {
                self.run_sequential(subtasks, executor, &event_tx).await?
            }
            ExecutionMode::Parallel => self.run_parallel(subtasks, executor, &event_tx).await?,
            ExecutionMode::Dag => {
                let waves = dag::build_dag_waves(&subtasks)?;
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

    async fn run_sequential(
        &self,
        mut subtasks: Vec<Subtask>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        let mut context = String::new();
        let mut results = Vec::new();

        for subtask in &mut subtasks {
            if *self.cancelled.lock().await {
                let _ = event_tx.send(OrchestratorEvent::TeamCancelled).await;
                anyhow::bail!("team cancelled");
            }

            let agent_id = format!("agent-{}", subtask.id);
            let _ = event_tx
                .send(OrchestratorEvent::AgentSpawned {
                    agent_id: agent_id.clone(),
                    agent_type: subtask.agent_type.clone(),
                    subtask_id: subtask.id.clone(),
                })
                .await;

            subtask.status = SubtaskStatus::Running;

            match self
                .execute_with_retry(subtask, &context, executor.as_ref())
                .await
            {
                Ok(result) => {
                    let _ = event_tx
                        .send(OrchestratorEvent::AgentComplete {
                            agent_id: agent_id.clone(),
                            subtask_id: subtask.id.clone(),
                            result: result.clone(),
                        })
                        .await;
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

    async fn run_parallel(
        &self,
        subtasks: Vec<Subtask>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
        let pool = AgentPool::new(self.config.max_concurrent);
        let mut handles = Vec::new();

        for subtask in subtasks {
            while !pool.can_spawn().await {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
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

            let exec = executor.clone();
            let pool_clone = pool.clone();
            let tx = event_tx.clone();
            let max_retries = self.config.max_retries;

            handles.push(tokio::spawn(async move {
                let result = retry_execute(&subtask, "", exec.as_ref(), max_retries).await;
                pool_clone.release(&agent_id).await;
                match result {
                    Ok(r) => {
                        let _ = tx
                            .send(OrchestratorEvent::AgentComplete {
                                agent_id,
                                subtask_id: subtask.id,
                                result: r.clone(),
                            })
                            .await;
                        Ok(r)
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

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(r)) => results.push(r),
                Ok(Err(e)) => results.push(format!("[FAILED: {e}]")),
                Err(e) => results.push(format!("[PANIC: {e}]")),
            }
        }

        Ok(results.join("\n---\n"))
    }

    async fn run_dag_waves(
        &self,
        subtasks: Vec<Subtask>,
        waves: Vec<Vec<String>>,
        executor: Arc<dyn SubtaskExecutor>,
        event_tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<String> {
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
                let context = subtask
                    .dependencies
                    .iter()
                    .filter_map(|dep_id| all_results.get(dep_id))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");

                let agent_id = format!("agent-{}", subtask.id);
                let _ = event_tx
                    .send(OrchestratorEvent::AgentSpawned {
                        agent_id: agent_id.clone(),
                        agent_type: subtask.agent_type.clone(),
                        subtask_id: subtask.id.clone(),
                    })
                    .await;

                let exec = executor.clone();
                let tx = event_tx.clone();
                let max_retries = self.config.max_retries;

                handles.push(tokio::spawn(async move {
                    let result =
                        retry_execute(&subtask, &context, exec.as_ref(), max_retries).await;
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

async fn retry_execute(
    subtask: &Subtask,
    context: &str,
    executor: &dyn SubtaskExecutor,
    max_retries: u32,
) -> anyhow::Result<String> {
    let mut last_err = String::new();
    for attempt in 0..=max_retries {
        match executor.execute(subtask, context).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_err = e.to_string();
                if attempt < max_retries {
                    tracing::warn!(
                        "subtask {} failed (attempt {}/{}): {e}",
                        subtask.id,
                        attempt + 1,
                        max_retries + 1
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        100 * u64::from(attempt + 1),
                    ))
                    .await;
                }
            }
        }
    }
    anyhow::bail!(
        "subtask '{}' failed after {} attempts: {}",
        subtask.id,
        max_retries + 1,
        last_err
    )
}
