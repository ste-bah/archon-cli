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
    // The other task does not survive even as an identity, and that is the
    // deliberate trade. It used to, because the whole universe was attached and
    // only the PROMPT was reduced; but `request.input` is walked recursively by
    // the enforcement paths, so every task left in it made this branch
    // answerable for that task's artifacts and tools. There is no way to keep a
    // foreign task's identity in the input without putting a foreign object in
    // reach of scanners that do not know whose it is, so the identity goes.
    //
    // Ordering information is not lost with it: the branch's OWN task still
    // carries its `dependency_ids` verbatim, so it still knows which ids it
    // depends on — it simply no longer receives those tasks' own entries.
    assert!(
        !rendered.contains("TASK-2"),
        "another task's entry reached a branch that does not claim it"
    );
    assert!(rendered.contains("TASK-1"), "the branch lost its own task");
}

/// The scoping that the three tests above depend on, pinned at the boundary the
/// enforcement paths actually read: `request.input`.
///
/// `project_artifact_contract::artifact_requirement_paths` and
/// `agent_adapter_a::collect_required_tool_names` both recurse the whole input
/// and harvest by KEY, with no notion of which task an entry belongs to. So a
/// foreign `artifact_requirements` or `required_tools` anywhere in the input is
/// enough to demote this branch's result for work it was never assigned — the
/// defect that made a Noop verdict unreachable and sent a real run into
/// noop-proof remediation instead of verification.
#[test]
fn an_implementation_branch_input_carries_no_other_tasks_contract() {
    let mut universe = universe();
    universe.tasks[0].artifact_requirements = vec!["artifacts/mine.json".to_string()];
    universe.tasks[0].required_tools = vec!["mine-tool".to_string()];
    universe.tasks[1].artifact_requirements = vec!["artifacts/not-mine.json".to_string()];
    universe.tasks[1].required_tools = vec!["not-mine-tool".to_string()];

    let request = v2_agent_request(
        "objective",
        None,
        &implementation_execution(),
        Some(&universe),
    );
    let input = request.input.to_string();

    assert!(
        input.contains("TASK-1") && input.contains("artifacts/mine.json"),
        "the branch's own declared contract never reached its input: {input}"
    );
    assert!(
        !input.contains("artifacts/not-mine.json"),
        "another task's artifact requirement reached this branch's input: {input}"
    );
    assert!(
        !input.contains("not-mine-tool"),
        "another task's required tool reached this branch's input: {input}"
    );
}

/// A branch whose claim resolves to nothing receives NO universe rather than
/// the whole one.
///
/// The fallback is the tempting shape and it is wrong: the moment the claim is
/// unknown is exactly the moment "give it everything" re-creates the original
/// defect, and it does so silently. An agent handed no contract is a gap
/// somebody can see; an agent judged against every task's contract fails in a
/// way that reads as its own error.
#[test]
fn a_branch_claiming_an_unresolvable_task_receives_no_universe() {
    for claim in [
        serde_json::json!({"item": {"canonical_task_ids": ["TASK-ABSENT"]}}),
        serde_json::json!({"item": {}}),
    ] {
        let mut execution = implementation_execution();
        execution.input = claim.clone();

        let request = v2_agent_request("objective", None, &execution, Some(&universe()));
        assert!(
            !request
                .input
                .to_string()
                .contains("workflow-v2-task-universe-v1"),
            "an unresolvable claim ({claim}) was answered with a universe"
        );
    }
}

/// A completion-claim repair still receives the whole set. Reconciling the
/// run's claims against the decomposition is legitimately about every task, and
/// it is a read-only call that declares no artifacts or tools of its own, so
/// there is nothing for the enforcement paths to over-apply.
#[test]
fn a_completion_claim_repair_still_receives_the_whole_universe() {
    let mut execution = implementation_execution();
    execution.call.id = "completion-claim-repair-1".to_string();
    execution.input = serde_json::json!({});

    let request = v2_agent_request("objective", None, &execution, Some(&universe()));
    let input = request.input.to_string();

    assert!(
        input.contains("TASK-1") && input.contains("TASK-2"),
        "a reconciliation call lost tasks it exists to reconcile: {input}"
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
