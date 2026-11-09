//! Agents the workflow host launches by a fixed key.
//!
//! A host call that names its own agent type must resolve in every project,
//! whatever that project's `.archon/agents` holds, or the launch fails with
//! "Unknown subagent type" before any work starts. These definitions are
//! registered after every on-disk source, so a project or user file of the
//! same name cannot redefine a host agent's tool set.

use super::definition::{AgentSource, CustomAgentDefinition};

/// The agent that re-authors one acceptance check the judge did not accept,
/// for `workflow freeze-acceptance --reauthor` and the in-round repair.
pub const ACCEPTANCE_REAUTHOR_AGENT: &str = "acceptance-reauthor";

/// The host agent that proposes requirement-directed changes.
pub const REQUIREMENT_BREAKER_AGENT: &str = "requirement-breaker";

/// The read-only tools a host agent may use: it observes, never writes or runs.
pub const HOST_READ_ONLY_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];

/// Whether `name` is a host agent: resolvable by key, never routed to a task.
pub fn is_host_agent(name: &str) -> bool {
    name == ACCEPTANCE_REAUTHOR_AGENT || name == REQUIREMENT_BREAKER_AGENT
}

/// Every host-launched agent definition.
pub fn get_harness_agents() -> Vec<CustomAgentDefinition> {
    vec![CustomAgentDefinition {
        agent_type: REQUIREMENT_BREAKER_AGENT.into(),
        description: "Host agent: proposes requirement-directed changes.".into(),
        system_prompt: "Observe the files in your working directory. Never run commands or write files. The task message defines the reply format.".into(),
        allowed_tools: Some(HOST_READ_ONLY_TOOLS.map(String::from).to_vec()),
        source: AgentSource::BuiltIn,
        ..Default::default()
    }, CustomAgentDefinition {
        agent_type: ACCEPTANCE_REAUTHOR_AGENT.into(),
        description: "Workflow host agent: re-authors one acceptance check entry. Launched by the host, not for direct use.".into(),
        system_prompt: concat!(
            "You re-author one acceptance check entry for a workflow task set. ",
            "Your tools are limited to Read, Grep and Glob: you observe the PRD and the repository, you never run commands or write files. ",
            "The task message defines the entry, the paths you may read and the exact reply format; follow it exactly.",
        )
        .into(),
        allowed_tools: Some(HOST_READ_ONLY_TOOLS.map(String::from).to_vec()),
        source: AgentSource::BuiltIn,
        ..Default::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentRegistry;

    #[test]
    fn acceptance_reauthor_resolves_in_a_project_with_no_agent_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
        let def = registry
            .resolve(ACCEPTANCE_REAUTHOR_AGENT)
            .expect("the host's re-author must resolve without any .archon/agents");
        assert_eq!(def.source, AgentSource::BuiltIn);
        assert_eq!(
            def.allowed_tools.as_deref(),
            Some(&HOST_READ_ONLY_TOOLS.map(String::from)[..])
        );
    }

    #[test]
    fn a_project_file_cannot_redefine_a_host_agent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let agents = tmp.path().join(".archon/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("acceptance-reauthor.md"),
            "---\nname: acceptance-reauthor\ndescription: overridden\ntools: Read, Write, Bash\n---\nWrite anything.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("probe-agent.md"),
            "---\nname: probe-agent\ndescription: probe\ntools: Read, Write, Bash\n---\nProbe.\n",
        )
        .unwrap();
        let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
        assert_eq!(
            registry.resolve("probe-agent").map(|def| &def.source),
            Some(&AgentSource::Project),
            "the same file shape loads as a project agent"
        );
        let def = registry.resolve(ACCEPTANCE_REAUTHOR_AGENT).unwrap();
        assert_eq!(def.source, AgentSource::BuiltIn);
        assert_eq!(
            def.allowed_tools.as_deref(),
            Some(&HOST_READ_ONLY_TOOLS.map(String::from)[..])
        );
    }

    #[test]
    fn a_host_agent_resolves_but_is_never_offered_for_routing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
        assert!(registry.resolve(ACCEPTANCE_REAUTHOR_AGENT).is_some());
        let names = registry.available_agent_names();
        assert!(!names.contains(&ACCEPTANCE_REAUTHOR_AGENT), "{names:?}");
        assert!(names.contains(&"general-purpose"), "{names:?}");
        for agent in get_harness_agents() {
            assert!(is_host_agent(&agent.agent_type));
        }
    }

    #[test]
    fn harness_agent_types_are_unique_and_disjoint_from_built_ins() {
        let mut names: Vec<String> = get_harness_agents()
            .into_iter()
            .map(|a| a.agent_type)
            .collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total);
        for built_in in crate::agents::built_in::get_built_in_agents() {
            assert!(!names.contains(&built_in.agent_type));
        }
    }
}

#[cfg(test)]
mod breaker_tests {
    use super::*;
    #[test]
    fn breaker_is_builtin_confined_memoryless_and_not_routed() {
        let root = tempfile::tempdir().unwrap();
        let agents = root.path().join(".archon/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("requirement-breaker.md"),"---\nname: requirement-breaker\ndescription: override\ntools: Bash, Write\n---\nOverride").unwrap();
        let registry = crate::agents::AgentRegistry::load_with_user_home(root.path(), None);
        let def = registry.resolve(REQUIREMENT_BREAKER_AGENT).unwrap();
        assert_eq!(def.source, AgentSource::BuiltIn);
        assert_eq!(
            def.allowed_tools,
            Some(HOST_READ_ONLY_TOOLS.map(String::from).to_vec())
        );
        assert!(def.memory_scope.is_none());
        assert!(def.recall_queries.is_empty());
        assert!(
            !registry
                .available_agent_names()
                .contains(&REQUIREMENT_BREAKER_AGENT)
        );
    }
}
