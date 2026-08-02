//! Wave resolution, context derivation, and retry — the pieces every runner in
//! [`crate::orchestrator`] shares.
//!
//! Split out of `orchestrator.rs` so that file stays inside the 500-line
//! preference after the O4 fix added agent-pool wiring to a second runner.

use std::collections::HashMap;

use super::SubtaskExecutor;
use super::events::Subtask;

/// Resolve wave membership back to subtasks.
///
/// Wave order first, then `TaskGraph::nodes` order within a wave, which the
/// lowering derives from the original `Vec<Subtask>` order. A decomposition
/// with no dependencies therefore flattens back to exactly the vector it came
/// from.
pub(super) fn flatten_waves(subtasks: &[Subtask], waves: &[Vec<String>]) -> Vec<Subtask> {
    waves
        .iter()
        .flatten()
        .filter_map(|id| subtasks.iter().find(|subtask| &subtask.id == id).cloned())
        .collect()
}

/// The results of a subtask's declared dependencies, joined.
///
/// Empty when nothing is declared — which is *unknown* dataflow rather than an
/// assertion that the task needs no input. Callers that have a better default
/// (the sequential path threads the previous result) must supply it themselves.
pub(super) fn dependency_context(subtask: &Subtask, completed: &HashMap<String, String>) -> String {
    subtask
        .dependencies
        .iter()
        .filter_map(|dependency| completed.get(dependency))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) async fn retry_execute(
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
