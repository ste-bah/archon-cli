use crate::gametheory::agents::{GameTheoryAgent, GameTheoryToolAccess};
use crate::gametheory::errors::GameTheoryError;
use crate::gametheory::facade::specialists::load_agent_prompt_body;


    #[test]
    fn load_agent_prompt_body_fails_when_declared_source_file_is_missing() {
        static ACCESS: &[GameTheoryToolAccess] = &[];
        let agent = GameTheoryAgent {
            key: "missing-source-agent",
            display_name: "Missing Source Agent",
            tier: 1,
            file: "missing-source-agent.md",
            memory_keys: &[],
            output_artifacts: &[],
            prompt_source_path: ".archon/agents/gametheory/does-not-exist-for-test.md",
            tool_access: ACCESS,
            model: "opus",
            condition: None,
            depends_on: &[],
            mandatory: false,
        };

        assert!(matches!(
            load_agent_prompt_body(&agent, std::path::Path::new("/tmp/nonexistent-project-root")),
            Err(GameTheoryError::MissingAgentFile { .. })
        ));
    }
