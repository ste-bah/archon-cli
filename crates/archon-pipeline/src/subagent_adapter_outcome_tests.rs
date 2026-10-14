use super::*;

#[test]
fn completed_subagent_without_provider_stop_reason_stays_missing() {
    let response = llm_response_for_subagent_outcome(
        SubagentOutcome::Completed(archon_tools::subagent_executor::SubagentCompletion {
            content: "body".into(),
            stop_reason: None,
            tokens_in: None,
            tokens_out: None,
        }),
        false,
        None,
    )
    .expect("a completed subagent is a successful turn");

    assert_eq!(response.content, "body");
    assert_eq!(response.stop_reason, None);
}

#[test]
fn completed_subagent_preserves_provider_usage_and_stop_reason() {
    let response = llm_response_for_subagent_outcome(
        SubagentOutcome::Completed(archon_tools::subagent_executor::SubagentCompletion {
            content: "partial body".into(),
            stop_reason: Some("max_tokens".into()),
            tokens_in: Some(321),
            tokens_out: Some(54),
        }),
        false,
        None,
    )
    .unwrap();
    assert_eq!(response.content, "partial body");
    assert_eq!(response.stop_reason.as_deref(), Some("max_tokens"));
    assert_eq!(response.tokens_in, Some(321));
    assert_eq!(response.tokens_out, Some(54));
}

#[test]
fn completed_subagent_stop_reason_evidence_covers_missing_and_provider_values() {
    for (case, before, expected) in [
        ("missing", None, None),
        ("provider-end-turn", Some("end_turn"), Some("end_turn")),
        (
            "provider-max-tokens",
            Some("max_tokens"),
            Some("max_tokens"),
        ),
    ] {
        let response = llm_response_for_subagent_outcome(
            SubagentOutcome::Completed(archon_tools::subagent_executor::SubagentCompletion {
                content: "observed reply".into(),
                stop_reason: before.map(str::to_string),
                tokens_in: None,
                tokens_out: None,
            }),
            false,
            None,
        )
        .unwrap();
        println!("EVIDENCE #402 {case} BEFORE: provider stop_reason={before:?}");
        println!(
            "EVIDENCE #402 {case} AFTER: surfaced stop_reason={:?}",
            response.stop_reason
        );
        assert_eq!(response.stop_reason.as_deref(), expected);
    }
}

#[test]
fn abnormal_subagent_endings_stay_errors_rather_than_typed_turns() {
    for (outcome, timed_out, needle) in [
        (
            SubagentOutcome::Failed("boom".into()),
            false,
            "subagent failed",
        ),
        (SubagentOutcome::Cancelled, true, "timed out"),
        (SubagentOutcome::Cancelled, false, "cancelled"),
        (
            SubagentOutcome::AutoBackgrounded,
            false,
            "auto-backgrounded",
        ),
    ] {
        let error = llm_response_for_subagent_outcome(outcome, timed_out, Some(7))
            .expect_err("an abnormal ending is not a turn");
        assert!(error.to_string().contains(needle), "{error}");
    }
}

#[test]
fn run_agent_returns_the_mapped_outcome_rather_than_building_a_response_inline() {
    let source = include_str!("subagent_adapter.rs");
    let execution = include_str!("subagent_adapter/continuation.rs");
    assert!(source.contains("self.execute_session(request, false).await"));
    assert!(
        execution.contains(
            "llm_response_for_subagent_outcome(outcome, timed_out, request.timeout_secs)"
        )
    );
    assert_eq!(execution.matches("stop_reason: None").count(), 0);
}
