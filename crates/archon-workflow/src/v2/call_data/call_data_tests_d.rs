//! An implementation branch is shown the contract it is judged against.
//!
//! Three things have to hold together, and each is useless without the others:
//! the universe must reach the request, the prompt gate must let the contract
//! through, and the bulky copy of the universe must be reduced so the criteria
//! that arrive are the branch's own rather than every task's.

use super::*;

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["project-tasks".to_string()],
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-1".to_string(),
                acceptance_criteria: vec!["the criterion this branch owns".to_string()],
                ..Default::default()
            },
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-2".to_string(),
                acceptance_criteria: vec!["a criterion belonging to another task".to_string()],
                ..Default::default()
            },
        ],
    }
}

fn implementation_execution() -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            // Deliberately the label a v3 script author writes, not an
            // engine-generated prefix: the decision must not depend on it.
            id: "remediate-tdl-020-1-0".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!({"item": {"canonical_task_ids": ["TASK-1"]}}),
        depends_on: Vec::new(),
    }
}

/// The universe reached the request at all. Without this the prompt layer has
/// nothing to digest and emits an empty contract block that reads exactly like
/// a task declaring nothing.
#[test]
fn an_implementation_branch_receives_the_task_universe() {
    let request = v2_agent_request(
        "objective",
        None,
        &implementation_execution(),
        Some(&universe()),
    );
    assert!(
        request
            .input
            .to_string()
            .contains("workflow-v2-task-universe-v1"),
        "no universe reached the implementation request"
    );
}

/// And the criteria actually arrive in the rendered prompt — the only place
/// that matters, since it is what the agent reads.
#[test]
fn an_implementation_branch_is_shown_the_criteria_it_is_judged_against() {
    let request = v2_agent_request(
        "objective",
        None,
        &implementation_execution(),
        Some(&universe()),
    );
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);
    let rendered = format!("{}\n{}", prompt.stable_prefix, prompt.invocation);

    assert!(
        rendered.contains("the criterion this branch owns"),
        "the branch never saw its own acceptance criteria"
    );
}

/// Scoped, not dumped. Attaching the universe without reducing it puts every
/// task's criteria in front of an agent answerable for one of them — on a real
/// fifteen-task decomposition that was a 118KB prompt, against 15KB scoped.
#[test]
fn an_implementation_branch_is_not_shown_another_tasks_criteria() {
    let request = v2_agent_request(
        "objective",
        None,
        &implementation_execution(),
        Some(&universe()),
    );
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);
    let rendered = format!("{}\n{}", prompt.stable_prefix, prompt.invocation);

    assert!(
        !rendered.contains("a criterion belonging to another task"),
        "another task's criteria leaked into the branch prompt"
    );
    // The other task still exists in the prompt as an identity, so dependency
    // and ownership reasoning survives the reduction.
    assert!(
        rendered.contains("TASK-2"),
        "the reduction erased the task graph"
    );
}

/// A read-only call is unchanged: it was never given the universe and must not
/// start receiving it as a side effect of this.
#[test]
fn a_read_only_call_still_receives_no_universe() {
    let mut execution = implementation_execution();
    execution.call.id = "inventory-1".to_string();
    execution.call.method = WorkflowV2HostMethod::Agent;
    execution.call.write_mode = None;

    let request = v2_agent_request("objective", None, &execution, Some(&universe()));
    assert!(
        !request
            .input
            .to_string()
            .contains("workflow-v2-task-universe-v1"),
        "a read-only call was given the universe"
    );
}
