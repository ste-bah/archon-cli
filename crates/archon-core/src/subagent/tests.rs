use super::*;

fn sample_request() -> SubagentRequest {
    SubagentRequest {
        prompt: "Analyze the codebase".into(),
        model: Some("claude-sonnet-4-6".into()),
        allowed_tools: vec!["Read".into(), "Glob".into()],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
    }
}

mod manager;
mod progress;
