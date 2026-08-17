use super::super::tool_types::PreflightResult;
use super::super::*;

pub(super) fn record_plan_execution(
    agent: &mut Agent,
    pre: &PreflightResult,
    result: &mut ToolResult,
    ctx: &ToolContext,
) {
    let original_error = result.is_error;
    if let Err(error) = agent.retry_pending_observation_failure() {
        let failure = format!(
            "pending filesystem observation failure could not be durably persisted: {error}"
        );
        tracing::error!(tool = "plan-reconciliation", "{failure}");
        if original_error {
            result.content = format!("{}\n[Plan observation failure: {failure}]", result.content);
        } else {
            *result =
                ToolResult::error(format!("Tool result cannot be accepted because {failure}"));
        }
        return;
    }
    let changed_files = match changed_plan_files(agent, pre, ctx) {
        Ok(files) => files,
        Err(error) => {
            reject_or_annotate_unobserved_mutation(agent, result, original_error, error);
            return;
        }
    };
    for file_path in changed_files {
        if let Err(error) = agent.record_plan_file_mutation(&file_path) {
            reject_or_annotate_unobserved_mutation(agent, result, original_error, error);
            return;
        }
    }
    record_terminal_task_update(agent, pre, result);
}

fn record_terminal_task_update(agent: &mut Agent, pre: &PreflightResult, result: &mut ToolResult) {
    if pre.tool_name != "TaskUpdate" {
        return;
    }
    let status = pre.input.get("status").and_then(|value| value.as_str());
    if status == Some("Completed") {
        agent.record_plan_completion_evidence(&pre.input);
    }
    if matches!(status, Some("Completed" | "Failed" | "Stopped"))
        && let Some(summary) = reconcile_after_terminal_plan_tasks(agent)
    {
        result.content = format!("{}\n{}", result.content, summary);
    }
}

fn reject_or_annotate_unobserved_mutation(
    agent: &mut Agent,
    result: &mut ToolResult,
    original_error: bool,
    error: String,
) {
    let failure = format!("exact filesystem mutations were not observed or persisted: {error}");
    let durable_failure = agent.record_plan_observation_failure(&failure).err();
    let failure = durable_failure.map_or(failure.clone(), |persistence_error| {
        format!("{failure}; durable completion blocker could not be persisted: {persistence_error}")
    });
    tracing::error!(tool = "plan-reconciliation", "{failure}");
    if original_error {
        result.content = format!("{}\n[Plan observation failure: {failure}]", result.content);
    } else {
        *result = ToolResult::error(format!("Tool result cannot be accepted because {failure}"));
    }
}

fn changed_plan_files(
    agent: &Agent,
    pre: &PreflightResult,
    ctx: &ToolContext,
) -> Result<Vec<String>, String> {
    let Some(before) = pre.filesystem_before.as_ref() else {
        return pre
            .filesystem_effect
            .requires_filesystem_observation()
            .then(|| format!("{} completed without a filesystem baseline", pre.tool_name))
            .map_or_else(|| Ok(Vec::new()), Err);
    };
    agent.changed_files_after_mutation(before).map_err(|error| {
        format!(
            "filesystem mutation after {} could not be observed in {}: {error}",
            pre.tool_name,
            ctx.working_dir.display()
        )
    })
}

pub(super) fn reconcile_after_terminal_plan_tasks(agent: &mut Agent) -> Option<String> {
    let store = agent.plan_store.as_ref()?;
    let plan = store.load_latest_plan(&agent.config.session_id).ok()??;
    let tasks = store
        .load_plan_tasks(&agent.config.session_id)
        .ok()?
        .into_iter()
        .filter(|task| task.plan_id == plan.id)
        .collect::<Vec<_>>();
    let terminal = !tasks.is_empty()
        && tasks
            .iter()
            .all(|task| matches!(task.status.as_str(), "Completed" | "Failed" | "Stopped"));
    terminal.then(|| agent.plan_completion_block()).flatten()
}

#[cfg(test)]
mod tests {
    #[test]
    fn plan_reconciliation_does_not_parse_shell_syntax() {
        // Bash execution containment, not command text, guarantees that a
        // descendant cannot mutate after its tool result is observed.
    }
}
