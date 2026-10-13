//! Issue-396 item 2: a read-only session whose tool calls keep returning the
//! same results is first asked for its deliverable with the inspection tools
//! withdrawn, and fails only when its next turn still answers with tools. A
//! write-capable session keeps the immediate stop.

use super::*;

/// Rounds of identical results that end a session: the novelty window fills
/// and warns at 8, then the tree may stay unchanged for 8 more rounds.
const ROUNDS_TO_STOP: usize = 16;

fn names(request: &LlmRequest) -> Vec<&str> {
    request
        .tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
        .collect()
}

fn missing_read(dir: &std::path::Path, n: usize) -> Vec<Vec<StreamEvent>> {
    let input = serde_json::json!({ "file_path": dir.join("missing-context.json") }).to_string();
    (0..n)
        .map(|index| tool_use_response(&format!("read-{index}"), "Read", &input))
        .collect()
}

fn runner_in(
    provider: Arc<MockProvider>,
    dir: &std::path::Path,
    session: &str,
    guard: archon_tools::workflow_read_guard::WorkflowReadGuard,
) -> SubagentRunner {
    let mut runner = make_runner(provider, 40);
    runner.tool_context.working_dir = dir.to_path_buf();
    runner.tool_context.session_id = session.into();
    runner.tool_context.subagent_id = Some(format!("{session}-agent"));
    runner.tool_context.workflow_read_guard = Some(Arc::new(guard));
    runner
}

fn read_only_guard() -> archon_tools::workflow_read_guard::WorkflowReadGuard {
    // Both inspection ceilings off: only the repeat stop can act here.
    archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
        &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
            read_only_soft_call_ceiling: 0,
            read_only_hard_call_ceiling: 0,
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn read_only_stall_withdraws_inspection_and_takes_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let mut responses = missing_read(dir.path(), ROUNDS_TO_STOP);
    responses.push(text_response("deliverable from what was read"));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = runner_in(
        provider.clone(),
        dir.path(),
        "stall-withdraw-answer",
        read_only_guard(),
    );

    let answer = runner.run("author the entry").await;

    assert_eq!(
        answer.expect("the stall asks for the answer instead of failing"),
        "deliverable from what was read"
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), ROUNDS_TO_STOP + 1);
    assert!(names(&requests[ROUNDS_TO_STOP - 1]).contains(&"Read"));
    let offered = names(&requests[ROUNDS_TO_STOP]);
    for &tool in archon_tools::workflow_read_guard::READ_ONLY_INSPECTION_TOOLS {
        assert!(!offered.contains(&tool), "{tool} still offered: {offered:?}");
    }
    let asked = serde_json::to_string(&requests[ROUNDS_TO_STOP].messages).unwrap();
    assert!(asked.contains("author the entry"), "{asked}");
    assert!(
        asked.contains("kept returning the same results"),
        "the note names why: {asked}"
    );
    assert!(
        asked.contains("Inspection tools are no longer available in this call; answer now with your deliverable from what you have read."),
        "{asked}"
    );
    assert!(
        asked.contains(archon_tools::workflow_read_guard::ANSWER_IN_TASK_FORMAT),
        "the note asks for the task's own deliverable format: {asked}"
    );
    assert!(!asked.contains(archon_tools::NO_PROGRESS_STOP_MARKER), "{asked}");
}

#[tokio::test]
async fn read_only_stall_still_fails_when_the_next_turn_calls_tools_again() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(MockProvider::new(missing_read(
        dir.path(),
        ROUNDS_TO_STOP + 1,
    )));
    let runner = runner_in(
        provider.clone(),
        dir.path(),
        "stall-withdraw-fail",
        read_only_guard(),
    );

    let error = runner
        .run("author the entry")
        .await
        .expect_err("a second stall after the withdrawal ends the session")
        .to_string();

    assert!(error.contains(archon_tools::NO_PROGRESS_STOP_MARKER), "{error}");
    assert_eq!(provider.requests().len(), ROUNDS_TO_STOP + 1);
}

#[tokio::test]
async fn write_capable_stall_keeps_the_immediate_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut responses = missing_read(dir.path(), ROUNDS_TO_STOP);
    responses.push(text_response("never asked"));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = runner_in(
        provider.clone(),
        dir.path(),
        "stall-write-capable",
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(1_000, 1_000, false, false),
    );

    let error = runner
        .run("edit the file")
        .await
        .expect_err("a write-capable stall still stops")
        .to_string();

    assert!(error.contains(archon_tools::NO_PROGRESS_STOP_MARKER), "{error}");
    assert_eq!(provider.requests().len(), ROUNDS_TO_STOP);
}

/// The live shape (body author, R8): after compaction the author paged its
/// own transcript with `read-own-evidence`. The same call repeated is
/// withdrawn at the first stall with an actionable instruction, long before
/// any inspection ceiling (here 120, the live value).
#[tokio::test]
async fn repeated_own_evidence_reads_are_withdrawn_before_the_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("own.jsonl");
    let mut lines: Vec<String> = (0..74).map(|line| format!("{{\"line\":{line}}}")).collect();
    // Short enough that sixteen copies stay below the compaction threshold:
    // a compaction request would take one of the scripted replies.
    lines.push(format!("{{\"content\":\"{}\"}}", "x".repeat(2_000)));
    std::fs::write(&transcript, lines.join("\n") + "\n").unwrap();
    let input = r#"{"offset":74,"limit":10}"#;
    let mut responses: Vec<_> = (0..ROUNDS_TO_STOP)
        .map(|index| tool_use_response(&format!("own-{index}"), "read-own-evidence", input))
        .collect();
    responses.push(text_response("---\ntask_id: TASK-X-001\n---\n# Body"));
    let provider = Arc::new(MockProvider::new(responses));
    let guard = archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
        &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
            read_only_soft_call_ceiling: 80,
            read_only_hard_call_ceiling: 120,
            ..Default::default()
        },
    );
    let mut runner = runner_in(provider.clone(), dir.path(), "stall-own-evidence", guard);
    let mut registry = (*runner.registry).clone();
    registry.replace(Box::new(super::super::evidence_recovery::ReadOwnEvidence(
        transcript,
    )));
    runner.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry = Arc::new(registry);

    let answer = runner.run("write the task body").await;

    assert_eq!(
        answer.expect("withdrawn and answered, not failed"),
        "---\ntask_id: TASK-X-001\n---\n# Body"
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), ROUNDS_TO_STOP + 1, "withdrawn after {ROUNDS_TO_STOP} of 120");
    assert!(names(&requests[0]).contains(&"read-own-evidence"));
    assert!(!names(&requests[ROUNDS_TO_STOP]).contains(&"read-own-evidence"));
    let asked = serde_json::to_string(&requests[ROUNDS_TO_STOP].messages).unwrap();
    assert!(asked.contains("kept returning the same results"), "{asked}");
    assert!(
        asked.contains(archon_tools::workflow_read_guard::ANSWER_IN_TASK_FORMAT),
        "{asked}"
    );
    let guard = runner.tool_context.workflow_read_guard.as_ref().unwrap();
    assert_eq!(guard.read_only_inspections(), ROUNDS_TO_STOP as u32);
    assert!(!guard.read_only_ceiling_reached());
}
