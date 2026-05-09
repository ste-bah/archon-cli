use archon_core::hooks::HookEvent;

#[test]
fn runtime_hook_events_accept_prd_snake_case_aliases() {
    let cases = [
        ("before_provider_resolve", HookEvent::BeforeProviderResolve),
        ("after_provider_resolve", HookEvent::AfterProviderResolve),
        ("before_prompt_build", HookEvent::BeforePromptBuild),
        ("after_prompt_build", HookEvent::AfterPromptBuild),
        ("before_agent_run", HookEvent::BeforeAgentRun),
        ("after_agent_run", HookEvent::AfterAgentRun),
        ("before_tool_call", HookEvent::BeforeToolCall),
        ("after_tool_call", HookEvent::AfterToolCall),
        ("before_learning_event", HookEvent::BeforeLearningEvent),
        ("after_learning_event", HookEvent::AfterLearningEvent),
        (
            "before_agent_profile_apply",
            HookEvent::BeforeAgentProfileApply,
        ),
        (
            "after_agent_profile_apply",
            HookEvent::AfterAgentProfileApply,
        ),
    ];

    for (raw, expected) in cases {
        let parsed: HookEvent = serde_json::from_str(&format!("\"{raw}\"")).unwrap();
        assert_eq!(parsed, expected);
    }
}
