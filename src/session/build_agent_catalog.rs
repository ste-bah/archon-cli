pub(super) fn common_inline_agents(agents: &[(String, String)]) -> Vec<(String, String)> {
    let mut selected = Vec::new();
    for wanted in [
        "sherlock-holmes",
        "general-purpose",
        "code-review",
        "reviewer",
        "researcher",
    ] {
        if let Some(agent) = agents.iter().find(|(name, _)| name == wanted)
            && !selected.iter().any(|(name, _)| name == &agent.0)
        {
            selected.push(agent.clone());
        }
    }
    for agent in agents {
        if selected.len() >= 20 {
            break;
        }
        if !selected.iter().any(|(name, _)| name == &agent.0) {
            selected.push(agent.clone());
        }
    }
    selected
}
