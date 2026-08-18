//! `TaskStore` over the global task manager (#189 Phase 9).
//!
//! `archon-tui` defines the trait and owns the overlay; it depends on
//! `archon-tools` only as a dev-dependency, so it cannot reach
//! `TASK_MANAGER` itself. This crate has both, which makes it the only place
//! the two halves can meet.
//!
//! Before this, `TASK_MANAGER`'s cancellation was reachable from the `TaskStop`
//! tool — that is, by the model — and from no key the user could press. The
//! overlay is the human half of the same capability.

use std::sync::Arc;
use std::time::SystemTime;

use archon_tools::background_agents::{
    RunningAgent, cancel_background_agent, running_background_agents,
};
use archon_tools::task_manager::{TASK_MANAGER, TaskInfo, TaskManager};
use archon_tui::{TaskId, TaskRow, TaskStore};

/// Prefixes that tell the two registries apart in the status column.
///
/// Both are listed because "which of these two things is my agent in" is not a
/// question a user should have to answer, and cancellation has to reach the
/// registry that actually holds the handle.
const TASK_KIND: &str = "task";
const AGENT_KIND: &str = "agent";

/// Reads and cancels across both process-global registries.
pub struct TaskManagerStore;

impl TaskManagerStore {
    /// Handle to inject into `AppConfig::task_store`.
    pub fn shared() -> Arc<dyn TaskStore> {
        Arc::new(Self)
    }
}

impl TaskStore for TaskManagerStore {
    fn list_tasks(&self) -> Vec<TaskRow> {
        merge_rows(&TASK_MANAGER.list_tasks(), &running_background_agents())
    }

    fn cancel_task(&self, id: &TaskId) -> Result<(), String> {
        // Try the task manager first; its ids are the ones `TaskStop` uses.
        // Only if it disowns the id is this a background agent.
        let Err(task_error) = TaskManager::stop_task(&TASK_MANAGER, id) else {
            return Ok(());
        };
        running_background_agents()
            .into_iter()
            .find(|agent| agent.subagent_id == *id)
            .map_or(Err(task_error), |agent| cancel_agent(&agent))
    }
}

/// Fold both registries into one ordered list.
///
/// A `TaskCreate` task and the agent it dispatched are the same work recorded
/// twice; `TaskInfo::agent_id` is the link between them. The agent row is the
/// one dropped, because the task id is what the user has already seen and what
/// `TaskStop` accepts.
fn merge_rows(tasks: &[TaskInfo], agents: &[RunningAgent]) -> Vec<TaskRow> {
    let dispatched: std::collections::HashSet<&str> =
        tasks.iter().filter_map(|t| t.agent_id.as_deref()).collect();
    let mut rows: Vec<TaskRow> = tasks.iter().map(task_row).collect();
    rows.extend(
        agents
            .iter()
            .filter(|agent| !dispatched.contains(agent.subagent_id.as_str()))
            .map(agent_row),
    );
    // Deterministic order. `TASK_MANAGER` stores tasks in a `HashMap` and the
    // agent registry in a `DashMap`, so an unsorted list reshuffles between
    // refreshes — and a cursor over a shuffling list cancels whatever happened
    // to land under it.
    rows.sort_by(|a, b| a.elapsed_secs.cmp(&b.elapsed_secs).then(a.id.cmp(&b.id)));
    rows
}

/// Cancel through the background-agent registry.
///
/// The registry is keyed by `subagent_id`, but its `cancel` resolves an
/// `AgentId` via `to_string()`. Those agree for the UUID-minting spawn paths —
/// `AgentTool` and `TaskCreate`, which is what this overlay lists — and differ
/// for `archon-pipeline`, whose ids are `{session}-{ordinal}-{agent}`. Rather
/// than pretend otherwise, a mismatch is reported as the limitation it is.
fn cancel_agent(agent: &RunningAgent) -> Result<(), String> {
    if agent.agent_id.to_string() != agent.subagent_id {
        return Err(format!(
            "{} is a pipeline agent; the registry cannot cancel it by runtime id",
            agent.subagent_id
        ));
    }
    cancel_background_agent(&agent.agent_id).map_err(|error| error.to_string())
}

/// Project a running agent onto the overlay's three columns.
fn agent_row(agent: &RunningAgent) -> TaskRow {
    let elapsed_secs = SystemTime::now()
        .duration_since(agent.spawned_at)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    TaskRow {
        id: agent.subagent_id.clone(),
        elapsed_secs,
        status: format!("{AGENT_KIND} · running"),
    }
}

/// Project a `TaskInfo` onto the three columns the overlay renders.
///
/// Elapsed is measured to completion for a finished task and to now for a live
/// one, so a stopped task's row stops advancing instead of implying it is still
/// burning time.
fn task_row(info: &TaskInfo) -> TaskRow {
    let end = info.completed_at.unwrap_or_else(chrono::Utc::now);
    let elapsed_secs = end
        .signed_duration_since(info.created_at)
        .num_seconds()
        .max(0) as u64;
    TaskRow {
        id: info.id.clone(),
        elapsed_secs,
        status: format!("{TASK_KIND} · {}", info.status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tools::task_manager::TaskStatus;
    use chrono::{Duration, Utc};

    fn info(id: &str, created_ago_secs: i64, completed_ago_secs: Option<i64>) -> TaskInfo {
        let now = Utc::now();
        TaskInfo {
            id: id.to_string(),
            description: "fixture".to_string(),
            status: TaskStatus::Running,
            created_at: now - Duration::seconds(created_ago_secs),
            completed_at: completed_ago_secs.map(|secs| now - Duration::seconds(secs)),
            output: String::new(),
            cost: 0.0,
            agent_id: None,
            board_item_id: None,
            metadata: None,
        }
    }

    #[test]
    fn elapsed_for_a_running_task_is_measured_to_now() {
        let row = task_row(&info("task-1", 90, None));
        assert!(
            (89..=92).contains(&row.elapsed_secs),
            "expected ~90s, got {}",
            row.elapsed_secs
        );
    }

    /// A finished task must stop accruing time, otherwise the overlay implies
    /// work is still running after it has stopped.
    #[test]
    fn elapsed_for_a_finished_task_freezes_at_completion() {
        let row = task_row(&info("task-1", 120, Some(60)));
        assert!(
            (59..=61).contains(&row.elapsed_secs),
            "expected ~60s, got {}",
            row.elapsed_secs
        );
    }

    #[test]
    fn clock_skew_cannot_produce_a_negative_elapsed() {
        let row = task_row(&info("task-1", 0, Some(30)));
        assert_eq!(row.elapsed_secs, 0);
    }

    /// The two registries must be tellable apart in the list, or the user
    /// cannot know which thing they are about to stop.
    #[test]
    fn rows_from_the_two_registries_are_visually_distinguished() {
        let task = task_row(&info("task-1", 10, None));
        let agent = agent_row(&RunningAgent {
            subagent_id: "agent-1".to_string(),
            agent_id: uuid::Uuid::new_v4(),
            spawned_at: SystemTime::now(),
        });

        assert!(task.status.starts_with("task ·"), "got {}", task.status);
        assert!(agent.status.starts_with("agent ·"), "got {}", agent.status);
        assert!(task.status.contains(&TaskStatus::Running.to_string()));
    }

    /// A pipeline agent's runtime id is not its `AgentId`, and the registry can
    /// only resolve the latter. Saying so beats reporting a bare "not found".
    #[test]
    fn a_pipeline_agent_reports_why_it_cannot_be_cancelled() {
        let agent = RunningAgent {
            subagent_id: "session-3-reviewer".to_string(),
            agent_id: uuid::Uuid::new_v4(),
            spawned_at: SystemTime::now(),
        };

        let error = cancel_agent(&agent).expect_err("pipeline id must not resolve");

        assert!(error.contains("pipeline agent"), "got {error}");
    }

    fn agent(subagent_id: &str, spawned_ago_secs: u64) -> RunningAgent {
        RunningAgent {
            subagent_id: subagent_id.to_string(),
            agent_id: uuid::Uuid::new_v4(),
            spawned_at: SystemTime::now() - std::time::Duration::from_secs(spawned_ago_secs),
        }
    }

    /// A `TaskCreate` task and the agent it dispatched are one piece of work.
    /// Listing it twice makes the user pick between two rows for the same
    /// thing, which is the confusion this overlay exists to end.
    #[test]
    fn a_task_and_the_agent_it_dispatched_appear_once() {
        let mut task = info("task-1", 30, None);
        task.agent_id = Some("agent-7".to_string());

        let rows = merge_rows(&[task], &[agent("agent-7", 30), agent("agent-9", 10)]);

        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["agent-9", "task-1"]);
    }

    #[test]
    fn an_unlinked_agent_still_appears() {
        let rows = merge_rows(&[info("task-1", 30, None)], &[agent("agent-9", 10)]);
        assert_eq!(rows.len(), 2);
    }

    /// Both registries iterate unordered maps. Without a total order the cursor
    /// lands on a different row each refresh, and `x` cancels the wrong thing.
    #[test]
    fn rows_are_totally_ordered_so_the_cursor_cannot_drift() {
        let rows = merge_rows(
            &[info("b", 10, None), info("a", 10, None)],
            &[agent("z", 5), agent("y", 5)],
        );

        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["y", "z", "a", "b"]);
    }

    #[test]
    fn an_unknown_id_is_refused_rather_than_silently_succeeding() {
        assert!(
            TaskManagerStore
                .cancel_task(&"no-such-id".to_string())
                .is_err()
        );
    }
}
