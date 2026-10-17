use std::path::Path;

use super::super::super::agents::GameTheoryAgent;
use super::super::super::errors::GameTheoryError;

pub fn load_agent_prompt_body(
    agent: &GameTheoryAgent,
    project_root: &Path,
) -> Result<String, GameTheoryError> {
    load_agent_prompt_body_at(agent, project_root)
}

pub(super) fn load_agent_prompt_body_at(
    agent: &GameTheoryAgent,
    root: &Path,
) -> Result<String, GameTheoryError> {
    crate::gametheory::load_agent_prompt_body(agent, root)
}
