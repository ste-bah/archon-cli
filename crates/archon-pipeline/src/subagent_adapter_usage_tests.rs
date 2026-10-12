use super::*;

#[test]
fn completed_subagent_preserves_missing_and_zero_usage() {
    let convert = |tokens_in, tokens_out| {
        llm_response_for_subagent_outcome(
            SubagentOutcome::Completed(archon_tools::subagent_executor::SubagentCompletion {
                content: "body".into(),
                stop_reason: None,
                tokens_in,
                tokens_out,
            }),
            false,
            None,
        )
        .unwrap()
    };
    let missing = convert(None, None);
    assert_eq!(missing.tokens_in, None);
    assert_eq!(missing.tokens_out, None);
    let zero = convert(Some(0), Some(0));
    assert_eq!(zero.tokens_in, Some(0));
    assert_eq!(zero.tokens_out, Some(0));
}
