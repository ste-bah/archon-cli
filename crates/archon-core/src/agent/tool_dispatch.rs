use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::events::emit_tool_result_activity;
use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn dispatch_allowed_tools(
        &self,
        allowed: &[PreflightResult],
        ctx: &ToolContext,
    ) -> Vec<ToolResult> {
        if allowed.len() > 1 && self.config.max_tool_concurrency > 1 {
            tracing::info!(
                tools = allowed.len(),
                max_concurrency = self.config.max_tool_concurrency,
                "dispatching tools concurrently"
            );
            let sem = Arc::new(Semaphore::new(self.config.max_tool_concurrency));
            let ctx_arc = Arc::new(ctx.clone());
            let mut join_set = JoinSet::new();

            for (idx, pre) in allowed.iter().enumerate() {
                let tool = pre.tool_arc.clone();
                let input = pre.input.clone();
                let ctx_clone = ctx_arc.clone();
                let sem_clone = sem.clone();
                let sandbox_prechecked = pre.sandbox_prechecked;

                join_set.spawn(async move {
                    let _permit = sem_clone.acquire().await.expect("semaphore closed");
                    // GHOST-006: sandbox pre-check (main-agent direct path).
                    let result = if sandbox_prechecked {
                        crate::dispatch::emit_tool_activity(
                            &ctx_clone,
                            tool.name(),
                            AgentActivityKind::ToolStarted,
                            AgentActivityStatus::Running,
                        );
                        let started_at = Instant::now();
                        let result = tool.execute(input, &ctx_clone).await;
                        emit_tool_result_activity(
                            &ctx_clone,
                            tool.name(),
                            &result,
                            started_at.elapsed(),
                        );
                        result
                    } else if let Some(ref backend) = ctx_clone.sandbox {
                        match backend.check(tool.name(), &input) {
                            Err(reason) => {
                                crate::dispatch::emit_tool_activity(
                                    &ctx_clone,
                                    tool.name(),
                                    AgentActivityKind::ToolFailed,
                                    AgentActivityStatus::Failed,
                                );
                                ToolResult::error(reason)
                            }
                            Ok(()) => {
                                crate::dispatch::emit_tool_activity(
                                    &ctx_clone,
                                    tool.name(),
                                    AgentActivityKind::ToolStarted,
                                    AgentActivityStatus::Running,
                                );
                                let started_at = Instant::now();
                                let result = tool.execute(input, &ctx_clone).await;
                                emit_tool_result_activity(
                                    &ctx_clone,
                                    tool.name(),
                                    &result,
                                    started_at.elapsed(),
                                );
                                result
                            }
                        }
                    } else {
                        crate::dispatch::emit_tool_activity(
                            &ctx_clone,
                            tool.name(),
                            AgentActivityKind::ToolStarted,
                            AgentActivityStatus::Running,
                        );
                        let started_at = Instant::now();
                        let result = tool.execute(input, &ctx_clone).await;
                        emit_tool_result_activity(
                            &ctx_clone,
                            tool.name(),
                            &result,
                            started_at.elapsed(),
                        );
                        result
                    };
                    (idx, result)
                });
            }

            let mut indexed: Vec<(usize, ToolResult)> = Vec::with_capacity(allowed.len());
            let mut panicked: Vec<ToolResult> = Vec::new();
            while let Some(join_result) = join_set.join_next().await {
                match join_result {
                    Ok(pair) => indexed.push(pair),
                    Err(e) => {
                        tracing::error!("tool task panicked: {e}");
                        panicked.push(ToolResult::error(format!("tool task panicked: {e}")));
                    }
                }
            }
            // Assign panicked results to the missing indices
            if !panicked.is_empty() {
                let seen: std::collections::HashSet<usize> =
                    indexed.iter().map(|(idx, _)| *idx).collect();
                let mut missing: Vec<usize> =
                    (0..allowed.len()).filter(|i| !seen.contains(i)).collect();
                for result in panicked {
                    let idx = missing.pop().unwrap_or(0);
                    indexed.push((idx, result));
                }
            }
            indexed.sort_by_key(|(idx, _)| *idx);
            indexed.into_iter().map(|(_, r)| r).collect()
        } else {
            // Sequential dispatch (single tool or concurrency disabled)
            let mut results = Vec::with_capacity(allowed.len());
            for pre in allowed {
                // GHOST-006: sandbox pre-check (main-agent sequential path).
                let result = if pre.sandbox_prechecked {
                    crate::dispatch::emit_tool_activity(
                        &ctx,
                        pre.tool_arc.name(),
                        AgentActivityKind::ToolStarted,
                        AgentActivityStatus::Running,
                    );
                    let started_at = Instant::now();
                    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                    emit_tool_result_activity(
                        &ctx,
                        pre.tool_arc.name(),
                        &result,
                        started_at.elapsed(),
                    );
                    result
                } else if let Some(ref backend) = ctx.sandbox {
                    match backend.check(pre.tool_arc.name(), &pre.input) {
                        Err(reason) => {
                            crate::dispatch::emit_tool_activity(
                                &ctx,
                                pre.tool_arc.name(),
                                AgentActivityKind::ToolFailed,
                                AgentActivityStatus::Failed,
                            );
                            ToolResult::error(reason)
                        }
                        Ok(()) => {
                            crate::dispatch::emit_tool_activity(
                                &ctx,
                                pre.tool_arc.name(),
                                AgentActivityKind::ToolStarted,
                                AgentActivityStatus::Running,
                            );
                            let started_at = Instant::now();
                            let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                            emit_tool_result_activity(
                                &ctx,
                                pre.tool_arc.name(),
                                &result,
                                started_at.elapsed(),
                            );
                            result
                        }
                    }
                } else {
                    crate::dispatch::emit_tool_activity(
                        &ctx,
                        pre.tool_arc.name(),
                        AgentActivityKind::ToolStarted,
                        AgentActivityStatus::Running,
                    );
                    let started_at = Instant::now();
                    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                    emit_tool_result_activity(
                        &ctx,
                        pre.tool_arc.name(),
                        &result,
                        started_at.elapsed(),
                    );
                    result
                };
                results.push(result);
            }
            results
        }
    }
}
