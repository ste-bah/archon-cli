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

impl GameTheoryToolAccess {
    pub fn tool_name(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Grep => "Grep",
            Self::Glob => "Glob",
            Self::Write => "Write",
            Self::WebSearch => "WebSearch",
            Self::WebFetch => "WebFetch",
        }
    }
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

impl GameTheoryAgent {
    pub fn allowed_tool_names(&self) -> Vec<String> {
        let mut tools: Vec<String> = self
            .tool_access
            .iter()
            .map(|tool| tool.tool_name().to_string())
            .collect();
        if !tools.iter().any(|tool| tool == "memory_recall") {
            tools.push("memory_recall".to_string());
        }
        tools
    }
}

/// A tier in the 12-tier game-theory organisation.
#[derive(Clone, Debug, Serialize)]
pub struct GameTheoryTier {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    pub agent_keys: &'static [&'static str],
}
