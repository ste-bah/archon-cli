//! Game-theory agent and tier type definitions.

use serde::Serialize;

/// Tool access capabilities for game-theory agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum GameTheoryToolAccess {
    Read,
    Grep,
    Glob,
    Write,
    WebSearch,
    WebFetch,
}

/// A single game-theory agent definition loaded from the curated arsenal.
#[derive(Clone, Debug, Serialize)]
pub struct GameTheoryAgent {
    pub key: &'static str,
    pub display_name: &'static str,
    pub tier: u8,
    pub file: &'static str,
    pub memory_keys: &'static [&'static str],
    pub output_artifacts: &'static [&'static str],
    pub prompt_source_path: &'static str,
    pub tool_access: &'static [GameTheoryToolAccess],
    pub model: &'static str,
    pub condition: Option<&'static str>,
    pub depends_on: &'static [&'static str],
    pub mandatory: bool,
}

/// A tier in the 12-tier game-theory organisation.
#[derive(Clone, Debug, Serialize)]
pub struct GameTheoryTier {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    pub agent_keys: &'static [&'static str],
}
