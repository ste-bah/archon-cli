//! After inspection is withdrawn, text-form tool calls are re-asked and the
//! session is not cut early: no stop beyond the existing no-progress rules
//! (the operator reserved a faster stop, #394). Live, R8
//! `acceptance-author-SUP-REQ-DL-124-100` answered eight withdrawals with a
//! text-form `Read` and delivered its entry on the ninth reply.

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

/// The SUP-REQ-DL-124-100 shape: eight text-form calls after the
/// withdrawal, each re-asked, then the deliverable.
#[tokio::test]
async fn eight_text_calls_after_the_withdrawal_then_the_deliverable_is_taken() {
    let entry = r#"{"id":"SUP-REQ-X-001","criterion":"c","check":{"kind":"command"}}"#;
    let mut responses = vec![tool_use_response(
        "read-0",
        "Read",
        r#"{"file_path":"Cargo.toml"}"#,
    )];
    responses.extend((0..8).map(|_| {
        text_response(
            "<tool_call>Read<arg_key>file_path</arg_key><arg_value>author-context/a.json</arg_value></tool_call>",
        )
    }));
    responses.push(text_response(entry));
    let provider = Arc::new(MockProvider::new(responses));
    let runner = ceiling_runner(provider.clone(), "withdrawn-dl124-shape");

    assert_eq!(
        runner
            .run("author the entry")
            .await
            .expect("the ninth reply delivers"),
        entry
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 10);
    for request in &requests[2..] {
        let last = request.messages.last().expect("a message");
        assert_eq!(last["role"], "user");
        assert!(
            last["content"]
                .as_str()
                .is_some_and(|text| text.starts_with(
                    "Inspection tools are no longer available in this call and text tool calls are not run"
                )),
            "{last}"
        );
    }
}
