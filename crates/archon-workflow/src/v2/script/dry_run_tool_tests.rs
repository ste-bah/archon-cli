//! Dry-run behaviour of `runTool` (#189 Phase 4).
//!
//! A dry run validates a script's shape without doing anything. That has to
//! stay true once scripts can call real tools, or validating a script becomes a
//! way to run commands — and the plan a dry run produces has to keep meaning
//! "the agent calls this script makes", which a `Read` is not.

use super::*;

async fn dry_run(script: &str) -> WorkflowResult<WorkflowDryRunPlanDetails> {
    dry_run_workflow_plan_full_details(script, None).await
}

/// The regression this phase must not cause: a script with no tool calls plans
/// exactly what it did before.
#[tokio::test]
async fn an_agent_only_script_plans_the_same_calls_as_before() {
    let details =
        dry_run("export default async function workflow(w) { await w.agent('research', {}); }")
            .await
            .expect("an agent-only script still validates");

    assert_eq!(details.calls.len(), 1);
    assert_eq!(details.calls[0].id, "research");
    assert!(
        !details.used_tool_calls,
        "a script that called no tool must not be marked as having done so"
    );
}

/// A tool call is answered, not executed, and does not enter the plan.
#[tokio::test]
async fn a_tool_call_is_not_executed_and_is_not_a_planned_call() {
    let details = dry_run(
        "export default async function workflow(w) { \
           await w.runTool('Bash', { command: 'echo this must not run' }); \
           await w.agent('research', {}); \
         }",
    )
    .await
    .expect("a script that calls a tool still validates");

    assert_eq!(
        details.calls.len(),
        1,
        "only the agent call belongs to the plan"
    );
    assert_eq!(details.calls[0].id, "research");
    assert!(details.used_tool_calls, "the tool call must be recorded");
}

/// The stand-in says what it is. A script that logs the result should show why
/// it is empty rather than implying the file was.
#[tokio::test]
async fn the_dry_run_stand_in_identifies_itself() {
    let details = dry_run(
        "export default async function workflow(w) { \
           const r = await w.runTool('Read', { file_path: 'x' }); \
           if (!r.content.includes('dry run')) { throw new Error('unmarked: ' + r.content); } \
           if (r.tool !== 'Read') { throw new Error('wrong tool: ' + r.tool); } \
           if (r.is_error) { throw new Error('a stand-in is not a failure'); } \
           await w.agent('research', {}); \
         }",
    )
    .await
    .expect("the stand-in carries its own explanation");

    assert!(details.used_tool_calls);
}

/// A script whose only host calls are tool calls plans no work, and that is an
/// error for the same reason an empty script is: there is nothing to run.
#[tokio::test]
async fn a_script_that_only_calls_tools_declares_no_executable_work() {
    let error = dry_run(
        "export default async function workflow(w) { await w.runTool('Read', { file_path: 'x' }); }",
    )
    .await
    .expect_err("tool calls alone are not a workflow");

    assert!(
        format!("{error}").contains("no executable host calls"),
        "{error}"
    );
}

/// Two calls to the same tool are two calls. The harness gives each a distinct
/// id from a counter rather than a random value, because the determinism rule
/// bans randomness — and without distinct ids the pending-call set would
/// collapse them into one.
#[tokio::test]
async fn repeat_calls_to_one_tool_do_not_collide() {
    let details = dry_run(
        "export default async function workflow(w) { \
           await w.runTool('Read', { file_path: 'a' }); \
           await w.runTool('Read', { file_path: 'b' }); \
           await w.agent('research', {}); \
         }",
    )
    .await
    .expect("two reads of different files are two calls, not a duplicate id");

    assert_eq!(details.calls.len(), 1);
    assert!(details.used_tool_calls);
}

#[tokio::test]
async fn a_tool_call_without_a_name_is_refused_by_the_harness() {
    let error = dry_run("export default async function workflow(w) { await w.runTool('', {}); }")
        .await
        .expect_err("a nameless tool call is a malformed script");

    assert!(format!("{error}").contains("runTool requires"), "{error}");
}
