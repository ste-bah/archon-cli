use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

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
                let tool_use_id = pre.tool_id.clone();
                let ctx_clone = ctx_arc.clone();
                let sem_clone = sem.clone();
                let sandbox_prechecked = pre.sandbox_prechecked;

                join_set.spawn(async move {
                    let _permit = sem_clone.acquire().await.expect("semaphore closed");
                    let attempt_ctx = ctx_clone.with_tool_run_attempt(tool_use_id, 0);
                    let result = crate::tool_run_admission::execute_tool_attempt(
                        tool.as_ref(),
                        input,
                        &attempt_ctx,
                        sandbox_prechecked,
                    )
                    .await;
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
                let attempt_ctx = ctx.with_tool_run_attempt(pre.tool_id.clone(), 0);
                let result = crate::tool_run_admission::execute_tool_attempt(
                    pre.tool_arc.as_ref(),
                    pre.input.clone(),
                    &attempt_ctx,
                    pre.sandbox_prechecked,
                )
                .await;
                results.push(result);
            }
            results
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tools::tool::{PermissionLevel, Tool, ToolRunAdmission};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DispatchTestTool {
        name: &'static str,
        executions: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for DispatchTestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "dispatch test"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            self.executions.fetch_add(1, Ordering::SeqCst);
            ToolResult::success("executed")
        }

        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Risky
        }
    }

    fn preflight(name: &'static str, id: &str, executions: &Arc<AtomicUsize>) -> PreflightResult {
        PreflightResult {
            tool_name: name.into(),
            tool_id: id.into(),
            input: serde_json::json!({}),
            tool_arc: Arc::new(DispatchTestTool {
                name,
                executions: Arc::clone(executions),
            }),
            file_path: None,
            sandbox_prechecked: true,
        }
    }

    fn blocking_context(seen: Arc<std::sync::Mutex<Vec<(String, u32)>>>) -> ToolContext {
        ToolContext {
            tool_run_parent_action_id: Some("parent-1".into()),
            tool_run_admission: Some(Arc::new(move |request| {
                seen.lock()
                    .unwrap()
                    .push((request.tool_use_id, request.attempt));
                ToolRunAdmission::Blocked {
                    reason: "blocked".into(),
                }
            })),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn sequential_dispatch_admits_each_tool_with_stable_id() {
        let mut agent = super::super::tests::test_agent();
        agent.config.max_tool_concurrency = 1;
        let executions = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let allowed = vec![preflight("RiskyA", "tool-a", &executions)];

        let results = agent
            .dispatch_allowed_tools(&allowed, &blocking_context(Arc::clone(&seen)))
            .await;

        assert!(results[0].is_error);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        assert_eq!(*seen.lock().unwrap(), vec![("tool-a".into(), 0)]);
    }

    #[tokio::test]
    async fn parallel_dispatch_admits_every_tool_with_its_stable_id() {
        let mut agent = super::super::tests::test_agent();
        agent.config.max_tool_concurrency = 2;
        let executions = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let allowed = vec![
            preflight("RiskyA", "tool-a", &executions),
            preflight("RiskyB", "tool-b", &executions),
        ];

        let results = agent
            .dispatch_allowed_tools(&allowed, &blocking_context(Arc::clone(&seen)))
            .await;

        assert!(results.iter().all(|result| result.is_error));
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        let mut seen = seen.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, vec![("tool-a".into(), 0), ("tool-b".into(), 0)]);
    }
}
