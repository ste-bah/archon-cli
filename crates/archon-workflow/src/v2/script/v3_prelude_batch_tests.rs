//! `agents()` batch concurrency: what the prelude asks the host for.
//!
//! Issue-53. The prelude used to clamp EVERY batch that mentioned `cargo ` to
//! one branch at a time, for the read-only case's reason (a shared checkout
//! shares one build lock). Write branches do not share one: each runs in its
//! own worktree and `apply_shell_roots` gives each repository path its own
//! build cache dir. In a Rust repository every task mentions cargo, so the
//! clamp serialised every write wave. These tests pin the request the prelude
//! makes; the host's `fanout_parallelism` still bounds it by the configured
//! subagent cap.

use super::super::dry_run_workflow_plan_full_details;
use crate::v2::WorkflowV2HostCall;

const META: &str = "export const meta = { name: 'x', description: 'y', phases: [] }\n";

async fn planned_calls(body: &str) -> Vec<WorkflowV2HostCall> {
    let script = format!("{META}{body}");
    let details = dry_run_workflow_plan_full_details(&script, None)
        .await
        .expect("a batch script plans");
    details.calls
}

fn spec(label: &str, prompt: &str, focused: &str) -> String {
    format!(
        "{{ prompt: {prompt:?}, label: {label:?}, taskIds: ['TASK-{label}'], \
           targetFiles: ['src/{label}.txt'], focusedTests: [{focused:?}] }}"
    )
}

/// The only call the batch made, so a stray second call fails loudly instead
/// of being skipped over by an index.
fn only_call(calls: &[WorkflowV2HostCall]) -> &WorkflowV2HostCall {
    assert_eq!(calls.len(), 1, "one batch, one host call: {calls:?}");
    &calls[0]
}

#[tokio::test]
async fn a_write_batch_that_mentions_cargo_keeps_its_requested_parallelism() {
    let body = format!(
        "await agents([{}, {}], {{ write: true, maxParallelism: 2 }})",
        spec("a", "Implement A.", "cargo test -p archon-workflow a_"),
        spec("b", "Implement B.", "cargo test -p archon-workflow b_"),
    );
    let calls = planned_calls(&body).await;
    let call = only_call(&calls);
    assert_eq!(
        call.write_mode,
        Some(crate::v2::WorkflowV2WriteMode::Worktree),
        "{call:?}"
    );
    assert_eq!(
        call.options.max_parallelism,
        Some(2),
        "worktree branches have their own build cache dir; mentioning cargo must not serialise them: {call:?}"
    );
}

#[tokio::test]
async fn a_read_only_batch_that_mentions_cargo_is_still_serialised() {
    let body = format!(
        "await agents([{}, {}], {{ maxParallelism: 2 }})",
        spec("a", "Run `cargo test` and report.", "echo a"),
        spec("b", "Inspect B.", "echo b"),
    );
    let calls = planned_calls(&body).await;
    let call = only_call(&calls);
    assert_eq!(call.write_mode, None, "{call:?}");
    assert_eq!(
        call.options.max_parallelism,
        Some(1),
        "read-only branches share the canonical checkout and its build lock: {call:?}"
    );
}

#[tokio::test]
async fn a_write_batch_without_cargo_passes_its_parallelism_through() {
    let body = format!(
        "await agents([{}, {}], {{ write: true, maxParallelism: 3 }})",
        spec("a", "Implement A.", "test -s src/a.txt"),
        spec("b", "Implement B.", "test -s src/b.txt"),
    );
    let calls = planned_calls(&body).await;
    assert_eq!(only_call(&calls).options.max_parallelism, Some(3));
}

/// No hint means no clamp either way: the host applies its own cap.
#[tokio::test]
async fn a_write_batch_with_no_hint_leaves_the_cap_to_the_host() {
    let body = format!(
        "await agents([{}], {{ write: true }})",
        spec("a", "Implement A.", "cargo test a_"),
    );
    let calls = planned_calls(&body).await;
    assert_eq!(only_call(&calls).options.max_parallelism, None);
}
