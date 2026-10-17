//! Semantic work signals carried by persisted decomposition calls.

pub(super) fn is_acceptance_evaluation_call(call: &archon_workflow::WorkflowV2CallRecord) -> bool {
    let role_is_judge = call
        .call
        .options
        .role
        .as_deref()
        .is_some_and(|role| role.eq_ignore_ascii_case("judge"));
    let command_is_acceptance = call
        .call
        .options
        .host_command
        .as_ref()
        .is_some_and(|command| {
            command
                .command_id
                .to_ascii_lowercase()
                .contains("acceptance")
        });
    let call_is_acceptance = call.call.id.to_ascii_lowercase().contains("acceptance");
    role_is_judge || command_is_acceptance || call_is_acceptance
}
