//! Concurrent tool execution with configurable parallelism.
//!
//! Independent tool calls can run in parallel, bounded by a semaphore.
//! Results are returned in the same order as the input calls.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::tool::{Tool, ToolContext, ToolResult};

/// A single tool call request.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Index to preserve ordering in the output.
    pub index: usize,
    /// The name of the tool to invoke.
    pub tool_name: String,
    /// JSON input for the tool.
    pub input: serde_json::Value,
}

/// Error returned when concurrent execution fails.
#[derive(Debug, thiserror::Error)]
pub enum ConcurrencyError {
    #[error("invalid max_concurrency {0}: must be 1..=16")]
    InvalidConcurrency(usize),

    #[error("tool '{0}' not found in registry")]
    ToolNotFound(String),

    #[error("task join error: {0}")]
    JoinError(String),
}

/// Default maximum number of concurrent tool calls.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// Minimum allowed concurrency.
const MIN_CONCURRENCY: usize = 1;

/// Maximum allowed concurrency.
const MAX_CONCURRENCY: usize = 16;

/// Result of a single tool call within a concurrent batch.
#[derive(Debug)]
pub struct IndexedToolResult {
    pub index: usize,
    pub tool_name: String,
    pub result: ToolResult,
}

/// Execute multiple tool calls concurrently with bounded parallelism.
///
/// Tools are resolved from `tools` (a map from name to `Arc<dyn Tool>`).
/// Unknown tools produce an error result rather than aborting the batch.
/// Results are returned sorted by the original `index` of each call.
///
/// # Arguments
/// * `calls` — the tool calls to execute
/// * `tools` — lookup function from tool name to `Arc<dyn Tool>`
/// * `ctx` — shared context for all calls
/// * `max_concurrency` — parallelism bound (1..=16)
pub async fn execute_tools_concurrent<F>(
    calls: Vec<ToolCall>,
    tool_lookup: F,
    ctx: Arc<ToolContext>,
    max_concurrency: usize,
) -> Result<Vec<IndexedToolResult>, ConcurrencyError>
where
    F: Fn(&str) -> Option<Arc<dyn Tool>> + Send + Sync + 'static,
{
    if !(MIN_CONCURRENCY..=MAX_CONCURRENCY).contains(&max_concurrency) {
        return Err(ConcurrencyError::InvalidConcurrency(max_concurrency));
    }

    if calls.is_empty() {
        return Ok(Vec::new());
    }

    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let tool_lookup = Arc::new(tool_lookup);
    let mut join_set = JoinSet::new();

    for call in calls {
        let sem = Arc::clone(&semaphore);
        let ctx = Arc::clone(&ctx);
        let lookup = Arc::clone(&tool_lookup);

        join_set.spawn(async move {
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    return IndexedToolResult {
                        index: call.index,
                        tool_name: call.tool_name,
                        result: ToolResult::error("concurrency semaphore closed unexpectedly"),
                    };
                }
            };

            let result = match lookup(&call.tool_name) {
                Some(tool) => tool.execute(call.input, &ctx).await,
                None => ToolResult::error(format!("unknown tool: '{}'", call.tool_name)),
            };

            IndexedToolResult {
                index: call.index,
                tool_name: call.tool_name,
                result,
            }
        });
    }

    let mut results = Vec::new();
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok(indexed) => results.push(indexed),
            Err(e) => return Err(ConcurrencyError::JoinError(e.to_string())),
        }
    }

    // Restore original ordering.
    results.sort_by_key(|r| r.index);

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{PermissionLevel, ToolContext};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "Echo"
        }
        fn description(&self) -> &str {
            "echoes input"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success(input.to_string())
        }
        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Safe
        }
    }

    /// Tracks peak concurrency to verify the semaphore works.
    struct SlowTool {
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "Slow"
        }
        fn description(&self) -> &str {
            "slow"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            // Record peak.
            self.peak.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            ToolResult::success("done")
        }
        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Safe
        }
    }

    fn test_ctx() -> Arc<ToolContext> {
        Arc::new(ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".into(),
            mode: crate::tool::AgentMode::Normal,
        })
    }

    #[tokio::test]
    async fn empty_calls_returns_empty() {
        let results = execute_tools_concurrent(
            vec![],
            |_| None,
            test_ctx(),
            DEFAULT_MAX_CONCURRENCY,
        )
        .await
        .expect("should succeed");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn results_returned_in_order() {
        let echo: Arc<dyn Tool> = Arc::new(EchoTool);
        let echo_clone = Arc::clone(&echo);

        let calls = (0..5)
            .map(|i| ToolCall {
                index: i,
                tool_name: "Echo".into(),
                input: serde_json::json!({"n": i}),
            })
            .collect();

        let results = execute_tools_concurrent(
            calls,
            move |name| {
                if name == "Echo" {
                    Some(Arc::clone(&echo_clone))
                } else {
                    None
                }
            },
            test_ctx(),
            2,
        )
        .await
        .expect("should succeed");

        assert_eq!(results.len(), 5);
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r.index, i);
            assert!(!r.result.is_error);
        }
    }

    #[tokio::test]
    async fn unknown_tool_produces_error_result() {
        let calls = vec![ToolCall {
            index: 0,
            tool_name: "DoesNotExist".into(),
            input: serde_json::json!({}),
        }];

        let results = execute_tools_concurrent(
            calls,
            |_| None,
            test_ctx(),
            DEFAULT_MAX_CONCURRENCY,
        )
        .await
        .expect("should succeed");

        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_error);
        assert!(results[0].result.content.contains("unknown tool"));
    }

    #[tokio::test]
    async fn semaphore_limits_concurrency() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let tool: Arc<dyn Tool> = Arc::new(SlowTool {
            active: Arc::clone(&active),
            peak: Arc::clone(&peak),
        });

        let calls = (0..8)
            .map(|i| ToolCall {
                index: i,
                tool_name: "Slow".into(),
                input: serde_json::json!({}),
            })
            .collect();

        let tool_clone = Arc::clone(&tool);
        let results = execute_tools_concurrent(
            calls,
            move |name| {
                if name == "Slow" {
                    Some(Arc::clone(&tool_clone))
                } else {
                    None
                }
            },
            test_ctx(),
            2,
        )
        .await
        .expect("should succeed");

        assert_eq!(results.len(), 8);
        // Peak concurrency should not exceed our limit of 2.
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn invalid_concurrency_rejected() {
        let result = execute_tools_concurrent(
            vec![],
            |_| None,
            test_ctx(),
            0,
        )
        .await;
        assert!(result.is_err());

        let result = execute_tools_concurrent(
            vec![],
            |_| None,
            test_ctx(),
            17,
        )
        .await;
        assert!(result.is_err());
    }
}
