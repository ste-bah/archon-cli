use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

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
    let mut join = archon_observability::spawn_named("subagent-executor", {
        let exec = Arc::clone(&exec);
        let cancel = cancel.clone();
        let ctx = ctx.clone();
        let req = request.clone();
        let system = system.clone();
        let sid = subagent_id.clone();
        async move {
            let result = exec
                .run_to_completion_with_system(sid, req, system, ctx, cancel.clone())
                .await;
            let cancelled = result.is_err() && cancel.is_cancelled();
            let execution = ExecutionResult { result, cancelled };
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
        SubagentOutcome::AutoBackgrounded => {}
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
