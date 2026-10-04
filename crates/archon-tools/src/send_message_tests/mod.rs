use super::*;

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-session-abc123".into(),
        mode: crate::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

/// A context that looks like a subagent's — `subagent_id` populated, session
/// shared with the parent. The distinction decides whether `lead` is a legal
/// address (#184 M1).
fn make_subagent_ctx() -> ToolContext {
    ToolContext {
        subagent_id: Some("subagent-child-1".into()),
        ..make_ctx()
    }
}

mod cases_a;
mod cases_b;
mod cases_lead;

/// A stopped agent is never resumed by message (#241); the guidance must not
/// promise it.
#[test]
fn the_tool_guidance_says_a_stopped_agent_is_refused_not_resumed() {
    let description = SendMessageTool.description();
    assert!(
        !description.contains("automatically resumed"),
        "{description}"
    );
    assert!(description.contains("start a new agent"), "{description}");
}
