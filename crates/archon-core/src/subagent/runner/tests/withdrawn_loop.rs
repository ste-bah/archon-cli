//! Issue 382 (and the live re-ask loop beside #394): once inspection is
//! withdrawn, a session that keeps answering with tool calls — written as
//! text and re-asked, or sent and refused — instead of its deliverable is
//! ended by the no-progress stop after the same round count the
//! repeated-results rule uses, with its refused-call count, not left to its
//! no-progress window for hours.

use super::*;

fn ceiling_runner(provider: Arc<MockProvider>, session: &str) -> SubagentRunner {
    let mut runner = make_runner(provider, 60);
    runner.tool_context.session_id = session.into();
    runner.tool_context.subagent_id = Some(format!("{session}-agent"));
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
            &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
                read_only_soft_call_ceiling: 0,
                read_only_hard_call_ceiling: 1,
                ..Default::default()
            },
        ),
    ));
    runner
}

fn first_read() -> Vec<StreamEvent> {
    tool_use_response("read-0", "Read", r#"{"file_path":"Cargo.toml"}"#)
}

fn text_call(offset: usize) -> Vec<StreamEvent> {
    text_response(&format!(
        "<tool_call>Grep<arg_key>offset</arg_key><arg_value>{offset}</arg_value></tool_call>"
    ))
}

#[tokio::test]
async fn text_form_calls_after_the_withdrawal_end_as_no_progress() {
    let mut responses = vec![first_read()];
    responses.extend((0..8).map(|turn| text_call(turn * 20)));
    responses.push(text_response("never reached"));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = ceiling_runner(provider.clone(), "withdrawn-text-loop");

    let error = runner
        .run("answer")
        .await
        .expect_err("eight unanswered turns end the session")
        .to_string();

    assert!(error.starts_with(archon_tools::NO_PROGRESS_STOP_MARKER), "{error}");
    assert!(error.contains("answered the next 8 turns"), "{error}");
    assert!(error.contains("last novel activity: turn 1: new tool call Read"), "{error}");
    assert!(error.contains("refused tool calls: 0"), "{error}");
    assert_eq!(provider.requests().len(), 9);
}

#[tokio::test]
async fn refused_calls_after_the_withdrawal_end_as_no_progress_with_their_count() {
    let mut responses = vec![first_read()];
    responses.extend((0..8).map(|turn| {
        tool_use_response(
            &format!("read-{}", turn + 1),
            "Read",
            &format!(r#"{{"file_path":"Cargo.toml","offset":{}}}"#, turn + 1),
        )
    }));
    responses.push(text_response("never reached"));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = ceiling_runner(provider.clone(), "withdrawn-refused-loop");

    let error = runner
        .run("answer")
        .await
        .expect_err("eight refused rounds end the session")
        .to_string();

    assert!(error.starts_with(archon_tools::NO_PROGRESS_STOP_MARKER), "{error}");
    assert!(error.contains("refused tool calls: 8"), "{error}");
    assert_eq!(provider.requests().len(), 9);
}

#[tokio::test]
async fn an_admitted_call_between_them_restarts_the_count() {
    let mut responses = vec![first_read()];
    responses.extend((0..7).map(|turn| text_call(turn * 20)));
    responses.push(tool_use_response(
        "build-1",
        "Bash",
        r#"{"command":"cargo --version"}"#,
    ));
    responses.extend((0..7).map(|turn| text_call(1_000 + turn * 20)));
    responses.push(text_response("the deliverable"));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = ceiling_runner(provider.clone(), "withdrawn-reset");

    assert_eq!(
        runner.run("answer").await.expect("an admitted build call is work"),
        "the deliverable"
    );
    assert_eq!(provider.requests().len(), 17);
}
