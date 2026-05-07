use crate::agents::definition::AgentMemoryScope;

/// Build the scoping tag for an agent type.
pub fn agent_tag(agent_type: &str) -> String {
    format!("agent:{agent_type}")
}

/// Build the scope isolation tag from an `AgentMemoryScope`.
pub fn scope_tag(scope: &AgentMemoryScope) -> String {
    match scope {
        AgentMemoryScope::User => "scope:user".to_string(),
        AgentMemoryScope::Project => "scope:project".to_string(),
        AgentMemoryScope::Local => "scope:local".to_string(),
    }
}
