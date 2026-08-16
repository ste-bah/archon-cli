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
