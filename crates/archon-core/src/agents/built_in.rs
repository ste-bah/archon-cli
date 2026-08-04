use super::definition::{AgentSource, CustomAgentDefinition, PermissionMode};

/// Returns the three built-in agents that are always available,
/// even without an `.archon/` directory on disk.
pub fn get_built_in_agents() -> Vec<CustomAgentDefinition> {
    vec![
        CustomAgentDefinition {
            agent_type: "general-purpose".into(),
            description: "General-purpose agent for research, code search, and multi-step tasks.".into(),
            system_prompt: include_str!("prompts/general_purpose.md").into(),
            allowed_tools: Some(
                vec!["Read", "Grep", "Glob", "Bash", "Write", "Edit"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            source: AgentSource::BuiltIn,
            ..Default::default()
        },
        CustomAgentDefinition {
            agent_type: "explore".into(),
            description: "Fast read-only exploration of codebases.".into(),
            system_prompt: include_str!("prompts/explore.md").into(),
            allowed_tools: Some(
                vec!["Read", "Grep", "Glob", "Bash"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            source: AgentSource::BuiltIn,
            ..Default::default()
        },
        CustomAgentDefinition {
            agent_type: "plan".into(),
            description: "Software architect for designing implementation plans.".into(),
            system_prompt: include_str!("prompts/plan.md").into(),
            allowed_tools: Some(
                vec!["Read", "Grep", "Glob"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            source: AgentSource::BuiltIn,
            ..Default::default()
        },
        CustomAgentDefinition {
            agent_type: "fork".into(),
            description: "Synthetic fork agent — inherits parent tools with Bubble permission.".into(),
            system_prompt: concat!(
                "You are a fork of the parent agent. You have access to all of the parent's tools.\n",
                "Complete the task described in the user message. Be thorough and precise.\n",
                "<fork_boilerplate>This agent is a fork child. Do not spawn nested forks.</fork_boilerplate>",
            ).into(),
            allowed_tools: None, // inherits parent's full tool set
            permission_mode: Some(PermissionMode::Bubble),
            max_turns: Some(200),
            model: None, // inherit parent model
            source: AgentSource::BuiltIn,
            omit_claude_md: true, // parent has full CLAUDE.md
            ..Default::default()
        },
    ]
}

/// Check if fork subagent mode is enabled via environment variable.
pub fn is_fork_enabled() -> bool {
    std::env::var("ARCHON_FORK_SUBAGENT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// FALLBACK fork guard: scan messages for the `<fork_boilerplate>` marker.
/// Returns true if the current context is already inside a fork child.
pub fn is_in_fork_child_by_messages(messages: &[serde_json::Value]) -> bool {
    messages.iter().any(|m| {
        // Check top-level "content" string
        if let Some(s) = m.get("content").and_then(|c| c.as_str())
            && s.contains("<fork_boilerplate>")
        {
            return true;
        }
        // Check content array entries (system prompt blocks)
        if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
            return arr.iter().any(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.contains("<fork_boilerplate>"))
                    .unwrap_or(false)
            });
        }
        false
    })
}

/// DUAL fork guard: PRIMARY checks agent_type == "fork", FALLBACK scans messages.
/// Both guards reject fork attempts inside fork children to prevent infinite recursion.
pub fn is_in_fork_child(resolved_agent_type: Option<&str>, messages: &[serde_json::Value]) -> bool {
    resolved_agent_type == Some("fork") || is_in_fork_child_by_messages(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_exactly_four_agents() {
        let agents = get_built_in_agents();
        assert_eq!(agents.len(), 4);
    }

    #[test]
    fn all_agents_are_builtin_source() {
        for agent in get_built_in_agents() {
            assert_eq!(agent.source, AgentSource::BuiltIn);
        }
    }

    #[test]
    fn all_agents_have_nonempty_system_prompt() {
        for agent in get_built_in_agents() {
            assert!(
                !agent.system_prompt.is_empty(),
                "agent {} has empty system_prompt",
                agent.agent_type
            );
        }
    }

    #[test]
    fn general_purpose_has_explicit_curated_tool_set() {
        let agents = get_built_in_agents();
        let gp = agents
            .iter()
            .find(|a| a.agent_type == "general-purpose")
            .unwrap();
        let expected: Vec<String> = ["Read", "Grep", "Glob", "Bash", "Write", "Edit"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(gp.allowed_tools.as_ref(), Some(&expected));
    }

    #[test]
    fn general_purpose_prompt_matches_curated_tool_set() {
        let agents = get_built_in_agents();
        let gp = agents
            .iter()
            .find(|a| a.agent_type == "general-purpose")
            .unwrap();

        assert!(!gp.system_prompt.contains("all available tools"));
        assert!(!gp.system_prompt.contains("web searches"));
        for capability in [
            "read files",
            "search codebases",
            "run commands",
            "edit files",
        ] {
            assert!(gp.system_prompt.contains(capability));
        }
    }

    #[test]
    fn explore_has_correct_tool_set() {
        let agents = get_built_in_agents();
        let explore = agents.iter().find(|a| a.agent_type == "explore").unwrap();
        let tools = explore.allowed_tools.as_ref().unwrap();
        assert_eq!(tools.len(), 4);
        assert!(tools.contains(&"Read".to_string()));
        assert!(tools.contains(&"Grep".to_string()));
        assert!(tools.contains(&"Glob".to_string()));
        assert!(tools.contains(&"Bash".to_string()));
    }

    #[test]
    fn plan_has_correct_tool_set() {
        let agents = get_built_in_agents();
        let plan = agents.iter().find(|a| a.agent_type == "plan").unwrap();
        let tools = plan.allowed_tools.as_ref().unwrap();
        assert_eq!(tools.len(), 3);
        assert!(tools.contains(&"Read".to_string()));
        assert!(tools.contains(&"Grep".to_string()));
        assert!(tools.contains(&"Glob".to_string()));
        assert!(!tools.contains(&"Bash".to_string()));
    }

    #[test]
    fn agent_types_are_unique() {
        let agents = get_built_in_agents();
        let mut types: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
        types.sort();
        types.dedup();
        assert_eq!(types.len(), 4);
    }

    #[test]
    fn all_agents_have_nonempty_description() {
        for agent in get_built_in_agents() {
            assert!(
                !agent.description.is_empty(),
                "agent {} has empty description",
                agent.agent_type
            );
        }
    }

    // -----------------------------------------------------------------------
    // Fork agent tests (AGT-023)
    // -----------------------------------------------------------------------

    #[test]
    fn fork_agent_has_bubble_permission() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert_eq!(fork.permission_mode, Some(PermissionMode::Bubble));
    }

    #[test]
    fn fork_agent_has_max_turns_200() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert_eq!(fork.max_turns, Some(200));
    }

    #[test]
    fn fork_agent_inherits_all_tools() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert!(
            fork.allowed_tools.is_none(),
            "fork should allow all tools (None)"
        );
    }

    #[test]
    fn fork_agent_inherits_parent_model() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert!(
            fork.model.is_none(),
            "fork should inherit parent model (None)"
        );
    }

    #[test]
    fn fork_agent_omits_claude_md() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert!(
            fork.omit_claude_md,
            "fork should omit CLAUDE.md (parent has it)"
        );
    }

    #[test]
    fn fork_agent_system_prompt_contains_boilerplate_marker() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert!(
            fork.system_prompt.contains("<fork_boilerplate>"),
            "fork system prompt must contain <fork_boilerplate> marker for guard"
        );
    }

    #[test]
    fn fork_agent_is_builtin_source() {
        let agents = get_built_in_agents();
        let fork = agents.iter().find(|a| a.agent_type == "fork").unwrap();
        assert_eq!(fork.source, AgentSource::BuiltIn);
    }

    // -----------------------------------------------------------------------
    // Fork guard tests (AGT-023)
    // -----------------------------------------------------------------------

    /// Serialises the four `ARCHON_FORK_SUBAGENT` tests against each other.
    ///
    /// They set and clear one process-wide variable, and the test harness runs
    /// them on parallel threads — so `defaults_to_false` could clear the
    /// variable between another test's `set_var` and its assertion. Observed as
    /// roughly one failure in ten full-suite runs, on whichever of the four lost
    /// the race.
    ///
    /// Poisoning is recovered from rather than propagated: the guard protects no
    /// data, and [`ForkEnvGuard`] restores the variable in `Drop`, which runs
    /// during the panic that poisoned the lock. Propagating would turn one real
    /// failure into three more that only report "mutex poisoned".
    static FORK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets `ARCHON_FORK_SUBAGENT` for one test and restores it afterwards.
    ///
    /// Restoring the previous value rather than clearing it means these tests do
    /// not change what the rest of the suite sees, even if the variable was set
    /// in the environment the run inherited.
    struct ForkEnvGuard {
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ForkEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let lock = FORK_ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var_os("ARCHON_FORK_SUBAGENT");
            match value {
                Some(value) => unsafe { std::env::set_var("ARCHON_FORK_SUBAGENT", value) },
                None => unsafe { std::env::remove_var("ARCHON_FORK_SUBAGENT") },
            }
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for ForkEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => unsafe { std::env::set_var("ARCHON_FORK_SUBAGENT", value) },
                None => unsafe { std::env::remove_var("ARCHON_FORK_SUBAGENT") },
            }
        }
    }

    #[test]
    fn is_fork_enabled_defaults_to_false() {
        let _guard = ForkEnvGuard::set(None);
        assert!(!is_fork_enabled());
    }

    #[test]
    fn is_fork_enabled_true_when_set() {
        let _guard = ForkEnvGuard::set(Some("1"));
        assert!(is_fork_enabled());
    }

    #[test]
    fn is_fork_enabled_true_when_set_to_true() {
        let _guard = ForkEnvGuard::set(Some("true"));
        assert!(is_fork_enabled());
    }

    #[test]
    fn is_fork_enabled_false_for_zero() {
        let _guard = ForkEnvGuard::set(Some("0"));
        assert!(!is_fork_enabled());
    }

    #[test]
    fn fork_guard_detects_marker_in_content_string() {
        let messages = vec![
            serde_json::json!({"role": "system", "content": "You are a fork. <fork_boilerplate>guard</fork_boilerplate>"}),
        ];
        assert!(is_in_fork_child_by_messages(&messages));
    }

    #[test]
    fn fork_guard_detects_marker_in_content_array() {
        let messages = vec![serde_json::json!({
            "role": "system",
            "content": [
                {"type": "text", "text": "normal text"},
                {"type": "text", "text": "<fork_boilerplate>guard</fork_boilerplate>"}
            ]
        })];
        assert!(is_in_fork_child_by_messages(&messages));
    }

    #[test]
    fn fork_guard_false_when_no_marker() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "Hello world"}),
            serde_json::json!({"role": "assistant", "content": "Hi there"}),
        ];
        assert!(!is_in_fork_child_by_messages(&messages));
    }

    #[test]
    fn fork_guard_false_for_empty_messages() {
        assert!(!is_in_fork_child_by_messages(&[]));
    }

    #[test]
    fn dual_fork_guard_primary_check() {
        // PRIMARY: agent_type == "fork" means we're inside a fork child
        assert!(is_in_fork_child(Some("fork"), &[]));
    }

    #[test]
    fn dual_fork_guard_fallback_check() {
        // FALLBACK: message scan detects marker even when agent_type is not "fork"
        let messages = vec![
            serde_json::json!({"role": "system", "content": "<fork_boilerplate>x</fork_boilerplate>"}),
        ];
        assert!(is_in_fork_child(None, &messages));
    }

    #[test]
    fn dual_fork_guard_neither_triggers_false() {
        assert!(!is_in_fork_child(Some("explore"), &[]));
    }
}
