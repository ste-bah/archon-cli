//! Issue 299: a script's tool calls are limited on NO-PROGRESS only, never
//! on totals. Many distinct calls and many bytes across calls succeed; one
//! oversized result is bounded in place; a loop that repeats the same call
//! and gets the same answer pauses the run before the call runs again.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const MIB: usize = 1024 * 1024;

/// A tool whose every execution is counted, so a test can prove which calls
/// actually ran. Input `bytes` sets the answer's size; `echo_count` makes the
/// answer change on every call; any other field only makes the call distinct.
#[derive(Debug)]
struct Fake(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl archon_tools::tool::Tool for Fake {
    fn name(&self) -> &str {
        "Fake"
    }

    fn description(&self) -> &str {
        "Counts its executions and answers with the requested number of bytes."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn capability(&self) -> archon_permissions::ToolCapability {
        archon_permissions::ToolCapability::EXECUTION
    }

    fn permission_level(&self, _input: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> archon_tools::tool::ToolResult {
        let count = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        if input["echo_count"] == serde_json::json!(true) {
            return archon_tools::tool::ToolResult::success(format!("call {count}"));
        }
        let bytes = input["bytes"].as_u64().unwrap_or(4) as usize;
        archon_tools::tool::ToolResult::success("x".repeat(bytes))
    }
}

fn fake_host(session: &str) -> (Arc<ScriptToolHost>, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let mut registry = archon_core::dispatch::ToolRegistry::new();
    registry.register(Box::new(Fake(runs.clone())));
    let host = ScriptToolHost {
        audited_writes: false,
        registry,
        checker: PermissionChecker::new(
            archon_permissions::mode::PermissionMode::default(),
            archon_permissions::rules::RuleSet {
                always_allow: vec![archon_permissions::rules::ToolRule {
                    tool: "Fake".to_string(),
                    pattern: "*".to_string(),
                }],
                always_deny: Vec::new(),
                always_ask: Vec::new(),
            },
        ),
        context: super::tests::context(session),
    };
    (Arc::new(host), runs)
}

fn call(id: usize, input: serde_json::Value) -> String {
    serde_json::json!({ "id": format!("Fake#{id}"), "options": { "name": "Fake", "input": input } })
        .to_string()
}

fn budget() -> Arc<std::sync::Mutex<ToolCallBudget>> {
    Arc::new(std::sync::Mutex::new(ToolCallBudget::default()))
}

fn answer(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("a tool answer is json")
}

/// The old total of 500 calls failed a script that read 501 distinct files.
#[tokio::test]
async fn five_hundred_and_one_distinct_calls_all_succeed() {
    let (host, runs) = fake_host("progress-distinct");
    let budget = budget();
    for i in 0..501 {
        let json = execute_run_tool(&host, &budget, &call(i, serde_json::json!({ "item": i })))
            .await
            .unwrap_or_else(|error| panic!("distinct call {} was refused: {error}", i + 1));
        assert_eq!(answer(&json)["is_error"], serde_json::json!(false));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 501);
}

/// The old 8 MiB total failed a script whose distinct reads summed past it.
#[tokio::test]
async fn more_than_eight_mib_across_distinct_calls_succeeds() {
    let (host, runs) = fake_host("progress-bytes");
    let budget = budget();
    let mut total = 0;
    for i in 0..5 {
        let input = serde_json::json!({ "item": i, "bytes": 2 * MIB });
        let json = execute_run_tool(&host, &budget, &call(i, input))
            .await
            .unwrap_or_else(|error| panic!("call {} was refused: {error}", i + 1));
        let content = answer(&json)["content"].as_str().unwrap_or_default().len();
        assert_eq!(
            content,
            2 * MIB,
            "a result under the per-call bound is whole"
        );
        total += content;
    }
    assert!(total > 8 * MIB, "{total}");
    assert_eq!(runs.load(Ordering::SeqCst), 5);
}

/// One huge result is bounded in place: the call ran once, its answer comes
/// back cut with a mark that says so, and the run goes on.
#[tokio::test]
async fn one_oversized_result_is_truncated_with_a_mark_and_never_rerun() {
    let (host, runs) = fake_host("progress-huge");
    let json = execute_run_tool(
        &host,
        &budget(),
        &call(1, serde_json::json!({ "bytes": 9 * MIB })),
    )
    .await
    .expect("an oversized answer is bounded, not fatal");
    let answer = answer(&json);
    let content = answer["content"].as_str().unwrap_or_default();
    assert!(content.len() < 9 * MIB, "{}", content.len());
    assert!(content.contains("truncated"), "the cut must be marked");
    assert_eq!(
        answer["truncated"],
        serde_json::json!(true),
        "{}",
        &content[content.len() - 200..]
    );
    assert_eq!(answer["original_bytes"], serde_json::json!(9 * MIB));
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the call ran exactly once");
}

/// A loop that repeats the same call and gets the same answer is the stall.
/// It pauses, and the call that would have repeated it again never runs.
#[tokio::test]
async fn a_loop_repeating_the_same_call_pauses_before_it_runs_again() {
    let (host, runs) = fake_host("progress-loop");
    let budget = budget();
    let mut answered = 0;
    let mut stop = None;
    for i in 0..1_000 {
        match execute_run_tool(
            &host,
            &budget,
            &call(i, serde_json::json!({ "path": "same" })),
        )
        .await
        {
            Ok(_) => answered += 1,
            Err(error) => {
                stop = Some(error);
                break;
            }
        }
    }
    let stop = stop.expect("a repeat loop must stop");
    assert!(
        matches!(&stop, WorkflowError::ControlPaused(message) if message.contains("Fake")),
        "a stall pauses, never fails: {stop:?}"
    );
    assert!(answered < 1_000, "{answered}");
    assert_eq!(
        runs.load(Ordering::SeqCst),
        answered,
        "the paused call was not executed and discarded"
    );
}

fn response(content: &str) -> RunToolResponse {
    RunToolResponse {
        tool: "Fake".to_string(),
        content: content.to_string(),
        is_error: false,
        truncated: false,
        original_bytes: None,
    }
}

/// The refusal carries the streak as evidence for the pause record.
#[test]
fn a_full_streak_refuses_the_next_identical_call_with_evidence() {
    let mut budget = ToolCallBudget::default();
    for i in 0..REPEAT_STALL_CALLS {
        assert!(
            budget
                .refuse_repeat(&format!("c{i}"), "Fake", "{}")
                .is_none()
        );
        budget.record(&format!("c{i}"), "Fake", "{}", &response("same"));
    }
    assert!(
        budget
            .refuse_repeat("other", "Fake", r#"{"a":1}"#)
            .is_none(),
        "a different call is not refused"
    );
    let message = budget
        .refuse_repeat("next", "Fake", "{}")
        .expect("the repeat is refused");
    assert!(message.contains("was not run"), "{message}");
    let evidence = budget.take_stall().expect("evidence is kept");
    assert_eq!(
        evidence["identical_calls_in_a_row"],
        serde_json::json!(REPEAT_STALL_CALLS)
    );
    assert_eq!(evidence["first_call_id"], "c0");
    assert_eq!(evidence["refused_call_id"], "next");
    assert_eq!(evidence["refused_call_executed"], false);
    assert!(budget.take_stall().is_none(), "taken once");
}

/// Progress resets the streak: a changed answer, changed arguments, or any
/// other host call in between.
#[test]
fn a_changed_answer_changed_arguments_or_other_work_resets_the_streak() {
    let mut budget = ToolCallBudget::default();
    let fill = |budget: &mut ToolCallBudget, n: usize, answer: &str| {
        for i in 0..n {
            budget.record(&format!("c{i}"), "Fake", "{}", &response(answer));
        }
    };
    fill(&mut budget, REPEAT_STALL_CALLS - 1, "one");
    fill(&mut budget, 1, "two");
    assert!(
        budget.refuse_repeat("x", "Fake", "{}").is_none(),
        "answer changed"
    );
    fill(&mut budget, REPEAT_STALL_CALLS - 1, "two");
    budget.record("y", "Fake", r#"{"b":2}"#, &response("two"));
    fill(&mut budget, 1, "two");
    assert!(
        budget.refuse_repeat("x", "Fake", "{}").is_none(),
        "arguments changed"
    );
    fill(&mut budget, REPEAT_STALL_CALLS, "two");
    budget.break_streak();
    assert!(
        budget.refuse_repeat("x", "Fake", "{}").is_none(),
        "other work happened"
    );
}

/// An answer that changes on every call is progress however often the same
/// arguments repeat, and no count of calls stops it.
#[tokio::test]
async fn identical_arguments_with_changing_answers_never_pause() {
    let (host, runs) = fake_host("progress-changing");
    let budget = budget();
    for i in 0..(REPEAT_STALL_CALLS * 3) {
        execute_run_tool(
            &host,
            &budget,
            &call(i, serde_json::json!({ "echo_count": true })),
        )
        .await
        .unwrap_or_else(|error| panic!("call {} was refused: {error}", i + 1));
    }
    assert_eq!(runs.load(Ordering::SeqCst), REPEAT_STALL_CALLS * 3);
}

/// The cut lands on a character boundary and a result at the bound is whole.
#[test]
fn the_per_call_bound_cuts_on_a_character_boundary() {
    let whole = progress::bound_response(response(&"x".repeat(MAX_RESULT_BYTES)));
    assert!(!whole.truncated);
    assert_eq!(whole.content.len(), MAX_RESULT_BYTES);
    let wide = "é".repeat(MAX_RESULT_BYTES / 2 + 1);
    let cut = progress::bound_response(response(&wide));
    assert!(cut.truncated);
    assert_eq!(cut.original_bytes, Some(wide.len()));
    assert!(cut.content.contains("tool result truncated"));
}
