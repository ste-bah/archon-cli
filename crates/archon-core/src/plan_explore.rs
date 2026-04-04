use archon_tools::agent_tool::SubagentRequest;

/// Read-only tools allowed for explore subagents.
const EXPLORE_TOOLS: &[&str] = &["Read", "Glob", "Grep", "ToolSearch"];

/// Default max turns for an explore subagent.
const EXPLORE_MAX_TURNS: u32 = 5;

/// Configuration for explore subagents.
///
/// Explore subagents are read-only: they may use `Read`, `Glob`, `Grep`,
/// and `ToolSearch` but never `Write`, `Edit`, or `Bash`.
#[derive(Debug, Clone)]
pub struct ExploreConfig {
    /// Tools the explore subagent is allowed to invoke.
    pub allowed_tools: Vec<String>,
    /// Maximum number of turns the explore subagent may take.
    pub max_turns: u32,
}

impl Default for ExploreConfig {
    fn default() -> Self {
        Self {
            allowed_tools: EXPLORE_TOOLS.iter().map(|s| (*s).into()).collect(),
            max_turns: EXPLORE_MAX_TURNS,
        }
    }
}

/// Result from an explore subagent run.
#[derive(Debug, Clone)]
pub struct ExploreResult {
    /// The original query that was explored.
    pub query: String,
    /// Free-form findings text returned by the subagent.
    pub findings: String,
    /// Paths of files the subagent read during exploration.
    pub files_read: Vec<String>,
    /// Total tokens consumed by the explore subagent.
    pub tokens_used: u64,
}

/// Create a [`SubagentRequest`] for an explore subagent.
///
/// The subagent is restricted to read-only tools and given a prompt
/// that instructs it to investigate the query without making changes.
pub fn create_explore_request(query: &str) -> SubagentRequest {
    let prompt = format!(
        "You are an explore subagent. Your job is to investigate the following question \
         using ONLY read-only tools (Read, Glob, Grep, ToolSearch). Do NOT modify any files.\n\n\
         Question: {query}\n\n\
         Report your findings clearly and concisely. List the files you examined."
    );

    SubagentRequest {
        prompt,
        model: None,
        allowed_tools: EXPLORE_TOOLS.iter().map(|s| (*s).into()).collect(),
        max_turns: EXPLORE_MAX_TURNS,
        timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
    }
}

/// Create a [`SubagentRequest`] from an [`ExploreConfig`] and query.
pub fn explore_config_to_request(config: &ExploreConfig, query: &str) -> SubagentRequest {
    let prompt = format!(
        "You are an explore subagent. Investigate this question using ONLY read-only tools. \
         Do NOT modify any files.\n\nQuestion: {query}\n\n\
         Report your findings clearly. List the files you examined."
    );

    SubagentRequest {
        prompt,
        model: None,
        allowed_tools: config.allowed_tools.clone(),
        max_turns: config.max_turns,
        timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
    }
}
