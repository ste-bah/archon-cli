const COMMON_INLINE_AGENT_TYPES: &[&str] = &[
    "general-purpose",
    "sherlock-holmes",
    "coder",
    "tester",
    "reviewer",
    "researcher",
    "planner",
    "plan",
    "explore",
    "code-reviewer",
    "code-review-swarm",
    "local-coder",
    "rust-systems-coder",
    "doc-writer",
    "frontend-implementation-specialist",
    "backend-implementation-specialist",
    "system-designer",
    "test-runner",
    "test-fixer",
    "security-tester",
];

pub(super) fn common_inline_agents(agents: &[(String, String)]) -> Vec<(String, String)> {
    let mut selected = Vec::new();
    for wanted in COMMON_INLINE_AGENT_TYPES {
        if let Some(agent) = agents.iter().find(|(name, _)| name == wanted)
            && !selected.iter().any(|(name, _)| name == &agent.0)
        {
            selected.push(agent.clone());
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str) -> (String, String) {
        (name.to_string(), format!("{name} description"))
    }

    fn names(agents: &[(String, String)]) -> Vec<&str> {
        agents.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn curated_inline_agents_keep_common_dev_agents_one_call() {
        let agents = vec![
            agent("abstract-writer"),
            agent("accessibility-specialist"),
            agent("active-exploitation-specialist"),
            agent("active-reconnaissance-specialist"),
            agent("coder"),
            agent("tester"),
            agent("reviewer"),
            agent("researcher"),
            agent("planner"),
            agent("plan"),
            agent("explore"),
            agent("code-reviewer"),
            agent("sherlock-holmes"),
            agent("general-purpose"),
        ];

        let selected = common_inline_agents(&agents);
        let names = names(&selected);

        for common in [
            "general-purpose",
            "sherlock-holmes",
            "coder",
            "tester",
            "reviewer",
            "researcher",
            "planner",
            "plan",
            "explore",
            "code-reviewer",
        ] {
            assert!(names.contains(&common), "{common} should stay inline");
        }
    }

    #[test]
    fn curated_inline_agents_do_not_alphabetically_pad_long_tail() {
        let agents = vec![
            agent("abstract-writer"),
            agent("academic-writer"),
            agent("accessibility-specialist"),
            agent("active-exploitation-specialist"),
        ];

        let selected = common_inline_agents(&agents);

        assert!(
            selected.is_empty(),
            "unlisted long-tail agents should be discovered through AgentCatalog"
        );
    }

    #[test]
    fn curated_inline_agents_follow_priority_order() {
        let agents = vec![
            agent("tester"),
            agent("sherlock-holmes"),
            agent("general-purpose"),
            agent("coder"),
        ];

        assert_eq!(
            names(&common_inline_agents(&agents)),
            vec!["general-purpose", "sherlock-holmes", "coder", "tester"]
        );
    }
}
