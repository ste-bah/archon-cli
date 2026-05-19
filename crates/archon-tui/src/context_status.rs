use crate::events::{AgentActivityStatus, AgentActivityUpdate};
use crate::status::StatusBar;

pub fn update_actor_context_name(status: &mut StatusBar, update: &AgentActivityUpdate) {
    match update.status {
        AgentActivityStatus::Running
        | AgentActivityStatus::Waiting
        | AgentActivityStatus::WaitingForTool => {}
        AgentActivityStatus::Complete
        | AgentActivityStatus::Failed
        | AgentActivityStatus::Cancelled => {
            if status.context_name.as_deref() == Some(update.name.as_str()) {
                status.context_name = Some("main".to_string());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{AgentActivityRole, AgentActivityUpdate};

    #[test]
    fn subagent_running_does_not_relabel_main_context_pressure() {
        let mut status = StatusBar {
            context_name: Some("main".into()),
            context_tokens_used: 121_000,
            ..StatusBar::default()
        };
        update_actor_context_name(
            &mut status,
            &AgentActivityUpdate {
                id: "agent-1".into(),
                name: "sherlock-holmes".into(),
                role: AgentActivityRole::Subagent,
                status: AgentActivityStatus::Running,
                current_tool: None,
                detail: None,
                run_id: None,
                parent_id: None,
                artifact_id: None,
                provider: None,
                model: None,
                cost_usd: None,
            },
        );

        assert_eq!(status.context_name.as_deref(), Some("main"));
    }
}
