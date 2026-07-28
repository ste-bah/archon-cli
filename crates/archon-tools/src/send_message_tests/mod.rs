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

mod cases_a;
mod cases_b;
