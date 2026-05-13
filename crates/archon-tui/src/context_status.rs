use crate::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use crate::status::StatusBar;

pub fn update_actor_context_name(status: &mut StatusBar, update: &AgentActivityUpdate) {
    match update.status {
        AgentActivityStatus::Running
        | AgentActivityStatus::Waiting
        | AgentActivityStatus::WaitingForTool => {
            if update.role != AgentActivityRole::Parent {
                status.context_name = Some(update.name.clone());
            }
        }
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
