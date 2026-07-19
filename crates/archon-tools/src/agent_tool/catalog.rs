use serde_json::json;

use super::request::AgentToolError;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const CATALOG_PAGE_LIMIT: usize = 25;

pub struct AgentCatalogTool {
    agents: Vec<(String, String)>,
}

impl AgentCatalogTool {
    pub fn new(mut agents: Vec<(String, String)>) -> Self {
        agents.sort_by(|a, b| a.0.cmp(&b.0));
        Self { agents }
    }

    fn capped_limit(input: &serde_json::Value) -> usize {
        input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, CATALOG_PAGE_LIMIT as u64) as usize)
            .unwrap_or(10)
    }

    fn entry_json((name, summary): &(String, String)) -> serde_json::Value {
        json!({
            "name": name,
            "description": summary,
        })
    }

    pub(super) fn list(&self, input: &serde_json::Value) -> serde_json::Value {
        let limit = Self::capped_limit(input);
        let page = input.get("page").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let start = page.saturating_mul(limit);
        let agents: Vec<_> = self
            .agents
            .iter()
            .skip(start)
            .take(limit)
            .map(Self::entry_json)
            .collect();
        json!({
            "action": "list",
            "page": page,
            "limit": limit,
            "total": self.agents.len(),
            "agents": agents,
        })
    }

    pub(super) fn search(&self, input: &serde_json::Value) -> serde_json::Value {
        let limit = Self::capped_limit(input);
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let agents: Vec<_> = self
            .agents
            .iter()
            .filter(|(name, summary)| {
                let name = name.to_ascii_lowercase();
                let summary = summary.to_ascii_lowercase();
                query.is_empty() || name.contains(&query) || summary.contains(&query)
            })
            .take(limit)
            .map(Self::entry_json)
            .collect();
        json!({ "action": "search", "query": query, "limit": limit, "agents": agents })
    }

    pub(super) fn info(
        &self,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, AgentToolError> {
        let name = input
            .get("name")
            .or_else(|| input.get("subagent_type"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(AgentToolError::MissingField("name"))?;
        let Some(entry) = self.agents.iter().find(|(agent, _)| agent == name) else {
            return Err(AgentToolError::InvalidInput(format!(
                "unknown agent '{name}'"
            )));
        };
        Ok(json!({ "action": "info", "agent": Self::entry_json(entry) }))
    }
}

#[async_trait::async_trait]
impl Tool for AgentCatalogTool {
    fn name(&self) -> &str {
        "AgentCatalog"
    }

    fn description(&self) -> &str {
        "List, search, and inspect available subagent types. Use this for agent discovery; use the Agent tool to launch a known subagent_type."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "search", "info"],
                    "description": "Catalog action to run."
                },
                "query": {
                    "type": "string",
                    "description": "Search text for action=search."
                },
                "name": {
                    "type": "string",
                    "description": "Agent type name for action=info."
                },
                "page": {
                    "type": "integer",
                    "description": "Zero-based page for action=list."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum rows to return, capped by Archon."
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        let result = match action {
            "list" => Ok(self.list(&input)),
            "search" => Ok(self.search(&input)),
            "info" => self.info(&input),
            other => Err(AgentToolError::InvalidInput(format!(
                "unknown AgentCatalog action '{other}'"
            ))),
        };
        match result {
            Ok(value) => ToolResult::success(value.to_string()),
            Err(err) => ToolResult::error(err.to_string()),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
