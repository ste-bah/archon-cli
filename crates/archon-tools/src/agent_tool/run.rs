use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::background_agents::{
    AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, RunRegistration, new_result_slot,
};
use crate::subagent_executor::{
    ExecutorError, SubagentExecutor, SubagentOutcome, get_subagent_executor,
};
use crate::subagent_request::SubagentRequest;
use crate::tool::ToolContext;

const FOREGROUND_CANCEL_CLEANUP_SECS: u64 = 30;

struct ExecutionResult {
    result: Result<String, ExecutorError>,
    cancelled: bool,
}

impl ExecutionResult {
    fn outcome(&self) -> SubagentOutcome {
        if self.cancelled {
            SubagentOutcome::Cancelled
        } else {
            match &self.result {
                Ok(text) => SubagentOutcome::Completed(text.clone()),
                Err(error) => SubagentOutcome::Failed(error.to_string()),
            }
        }
    }

    fn terminal_status(&self) -> AgentStatus {
        if self.cancelled {
            AgentStatus::Cancelled
        } else if self.result.is_ok() {
            AgentStatus::Finished
        } else {
            AgentStatus::Failed
        }
    }
}

/// Marks a subagent as running in `BACKGROUND_AGENTS` for exactly as long as
/// its runner lives.
///
/// This exists so liveness is a property of *having been spawned* rather than
/// something each spawn path has to remember. Every public runner in this
/// module funnels through `run_subagent_with_auto_background`, so registering
/// there covers `AgentTool`, `TaskCreate`, `archon-pipeline`, `message_delivery`
/// and whatever is added next, and `board::leases::holder_liveness` gets to be
/// one lookup instead of a fan-out that grows with the spawn paths.
///
/// Release is a `Drop` rather than a call at the end of the runner because the
/// runner does not always reach its end: it can panic, and
/// `await_cancelled_foreground` aborts it outright after the cleanup grace
/// period. A leaked `Running` entry parks a board claim for the life of the
/// process, which is the failure the lease was built to prevent.
struct SpawnedAgent {
    subagent_id: String,
    outcome: AgentStatus,
}

impl SpawnedAgent {
    fn register(subagent_id: &str, cancel: &CancellationToken) -> Self {
        let handle = BackgroundAgentHandle {
            // Only the UUID-minting spawn paths have a meaningful `agent_id`;
            // for the rest the identity that matters is `subagent_id`, which is
            // what the registry is keyed by.
            agent_id: Uuid::parse_str(subagent_id).unwrap_or_else(|_| Uuid::new_v4()),
            subagent_id: subagent_id.to_string(),
            // A foreground run has no spawned task to hand over; the field is
            // already an `Option` for exactly this.
            join_handle: None,
            cancel_token: cancel.clone(),
            spawned_at: SystemTime::now(),
            status: Arc::new(Mutex::new(AgentStatus::Running)),
            result_slot: new_result_slot(),
        };
        // Registering an id the registry has already seen is a defined
        // outcome, not an error: `AgentTool` registers on the parent task too
        // (so `execute`'s spawn marker is truthful), and `SendMessage` resumes
        // an agent under its original id, which may or may not still be in the
        // registry depending on whether the reaper has run. `register_run`
        // revives a terminal entry for exactly that reason.
        match BACKGROUND_AGENTS.register_run(handle) {
            RunRegistration::Registered | RunRegistration::AlreadyRunning => {}
            RunRegistration::Restarted => {
                tracing::debug!(
                    subagent_id = %subagent_id,
                    "subagent id resumed; replacing the terminal registry entry"
                );
            }
        }
        Self {
            subagent_id: subagent_id.to_string(),
            // Overwritten by `finished` on every path that produces an outcome.
            // What is left is a runner that was torn down without one.
            outcome: AgentStatus::Cancelled,
        }
    }

    fn finished(&mut self, outcome: AgentStatus) {
        self.outcome = outcome;
    }
}

impl Drop for SpawnedAgent {
    fn drop(&mut self) {
        BACKGROUND_AGENTS.mark_terminal(&self.subagent_id, self.outcome);
    }
}

// ---------------------------------------------------------------------------
// run_subagent — the AGT-025 `tokio::select!` race, relocated from
// archon-core per TASK-AGS-105 mapping doc Section 2c.
// ---------------------------------------------------------------------------
//
// Owns the AGT-025 auto-background race against the installed
// `SubagentExecutor`. The executor's `run_to_completion` fires
// `on_inner_complete` at its tail UNCONDITIONALLY (preserves
// PRESERVE-D8). `run_subagent` fires `on_visible_complete` only on the
// non-timer arms (preserves PRESERVE-D5 — post-abandonment auto-bg
// agents get inner side effects but NOT visible hooks).
pub async fn run_subagent(
    subagent_id: String,
    request: SubagentRequest,
    cancel: CancellationToken,
    ctx: ToolContext,
) -> SubagentOutcome {
    run_subagent_with_auto_background(subagent_id, request, Vec::new(), cancel, ctx, true, None)
        .await
}

pub async fn run_subagent_with_system(
    subagent_id: String,
    request: SubagentRequest,
    system: Vec<serde_json::Value>,
    cancel: CancellationToken,
    ctx: ToolContext,
) -> SubagentOutcome {
    run_subagent_with_auto_background(subagent_id, request, system, cancel, ctx, true, None).await
}

pub(crate) async fn run_subagent_with_completion(
    subagent_id: String,
    request: SubagentRequest,
    cancel: CancellationToken,
    ctx: ToolContext,
    auto_background_completion: oneshot::Sender<SubagentOutcome>,
) -> SubagentOutcome {
    run_subagent_with_auto_background(
        subagent_id,
        request,
        Vec::new(),
        cancel,
        ctx,
        true,
        Some(auto_background_completion),
    )
    .await
}

/// Run a subagent as an awaited foreground operation even when the global
/// auto-background env gate is enabled. Generated workflow host calls use this
/// because `workflow.js` is awaiting a concrete result for resume/fanin.
pub async fn run_subagent_foreground(
    subagent_id: String,
    request: SubagentRequest,
    cancel: CancellationToken,
    ctx: ToolContext,
) -> SubagentOutcome {
    run_subagent_with_auto_background(subagent_id, request, Vec::new(), cancel, ctx, false, None)
        .await
}

pub async fn run_subagent_foreground_with_system(
    subagent_id: String,
    request: SubagentRequest,
    system: Vec<serde_json::Value>,
    cancel: CancellationToken,
    ctx: ToolContext,
) -> SubagentOutcome {
    run_subagent_with_auto_background(subagent_id, request, system, cancel, ctx, false, None).await
}

async fn run_subagent_with_auto_background(
    subagent_id: String,
    request: SubagentRequest,
    system: Vec<serde_json::Value>,
    cancel: CancellationToken,
    ctx: ToolContext,
    allow_auto_background: bool,
    auto_background_completion: Option<oneshot::Sender<SubagentOutcome>>,
) -> SubagentOutcome {
    let exec = match get_subagent_executor() {
        Some(e) => e,
        None => {
            return SubagentOutcome::Failed("subagent executor not installed".to_string());
        }
    };
    let auto_bg_ms = if allow_auto_background {
        exec.auto_background_ms()
    } else {
        0
    };

    let nested = ctx.nested;
    // Registered here, on the caller's task, so the entry exists from the
    // instant the runner is spawned; the guard is then moved into the runner so
    // that the release happens when the *runner* ends, not when this function
    // returns. Those are different moments on the `AutoBackgrounded` arm, and
    // the agent still working past that arm is precisely the one whose claim
    // must not be swept.
    let alive = SpawnedAgent::register(&subagent_id, &cancel);
    let mut join = archon_observability::spawn_named("subagent-executor", {
        let exec = Arc::clone(&exec);
        let cancel = cancel.clone();
        let ctx = ctx.clone();
        let req = request.clone();
        let system = system.clone();
        let sid = subagent_id.clone();
        async move {
            let mut alive = alive;
            let result = exec
                .run_to_completion_with_system(sid, req, system, ctx, cancel.clone())
                .await;
            let cancelled = result.is_err() && cancel.is_cancelled();
            let execution = ExecutionResult { result, cancelled };
            alive.finished(execution.terminal_status());
            if let Some(completion) = auto_background_completion {
                let _ = completion.send(execution.outcome());
            }
            execution
        }
    });

    let outcome = if auto_bg_ms == 0 {
        tokio::select! {
            biased;
            r = &mut join => match r {
                Ok(execution) => execution.outcome(),
                Err(e) => SubagentOutcome::Failed(format!("join panic: {e}")),
            },
            _ = cancel.cancelled() => {
                await_cancelled_foreground(&mut join, exec.as_ref(), &subagent_id).await
            },
        }
    } else {
        let timer = tokio::time::sleep(Duration::from_millis(auto_bg_ms));
        tokio::select! {
            biased;
            r = &mut join => match r {
                Ok(execution) => execution.outcome(),
                Err(e) => SubagentOutcome::Failed(format!("join panic: {e}")),
            },
            _ = cancel.cancelled() => {
                await_cancelled_foreground(&mut join, exec.as_ref(), &subagent_id).await
            },
            _ = timer => SubagentOutcome::AutoBackgrounded,
        }
    };

    // on_visible_complete fires ONLY for non-timer completion arms.
    // The AutoBackgrounded arm INTENTIONALLY does NOT call it, which
    // preserves PRESERVE-D5: post-abandonment auto-backgrounded agents
    // get inner side effects (fired from run_to_completion's tail when
    // the runner eventually finishes) but NOT visible hooks or
    // worktree cleanup.
    match &outcome {
        SubagentOutcome::Completed(text) => {
            let side_effects = exec
                .on_visible_complete(subagent_id.clone(), Ok(text.clone()), nested)
                .await;
            // If there's a worktree-preserved note, splice it into the
            // returned text. The executor returned the base text via
            // run_to_completion; we append the suffix here so the
            // caller (AgentTool::execute) receives the fully-composed
            // string with no awareness of worktree plumbing.
            if let Some(suffix) = side_effects.text_suffix {
                return SubagentOutcome::Completed(format!("{text}{suffix}"));
            }
        }
        SubagentOutcome::Failed(err) => {
            let _ = exec
                .on_visible_complete(subagent_id.clone(), Err(err.clone()), nested)
                .await;
        }
        // Still no visible hooks and no worktree cleanup — PRESERVE-D5 holds.
        // What does happen is a status signal to the lead, which acts on
        // nothing and is the only thing separating "wedged" from "busy" once
        // the timer has abandoned the join. Synchronous by contract: this arm
        // must not await, and `preserve_d5_agt025.rs` reads this file as text
        // to enforce it (#184 M6).
        SubagentOutcome::AutoBackgrounded => {
            exec.on_auto_backgrounded(&subagent_id);
        }
        SubagentOutcome::Cancelled => {
            let _ = exec
                .on_visible_complete(
                    subagent_id.clone(),
                    Err("subagent cancelled".to_string()),
                    nested,
                )
                .await;
        }
    }

    outcome
}

async fn await_cancelled_foreground(
    join: &mut tokio::task::JoinHandle<ExecutionResult>,
    exec: &dyn SubagentExecutor,
    subagent_id: &str,
) -> SubagentOutcome {
    match tokio::time::timeout(
        Duration::from_secs(FOREGROUND_CANCEL_CLEANUP_SECS),
        &mut *join,
    )
    .await
    {
        Ok(Ok(_)) => SubagentOutcome::Cancelled,
        Ok(Err(err)) => SubagentOutcome::Failed(format!("join panic: {err}")),
        Err(_) => {
            join.abort();
            let _ = join.await;
            exec.on_inner_complete(
                subagent_id.to_string(),
                Err("subagent cancelled".to_string()),
            )
            .await;
            SubagentOutcome::Cancelled
        }
    }
}
