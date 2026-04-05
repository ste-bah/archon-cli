//! SendMessage tool for agent teams (TASK-CLI-312).
//!
//! Appends a message to one or all member inboxes in a file-based team.

use std::io::Write;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::team_config::TeamConfig;
use crate::team_message::{MessageType, TeamMessage};

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct SendMessageTeamTool {
    project_dir: PathBuf,
}

impl SendMessageTeamTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn team_dir(&self, team_id: &str) -> PathBuf {
        self.project_dir.join(".claude").join("teams").join(team_id)
    }

    fn inbox_path(&self, team_id: &str, role: &str) -> PathBuf {
        self.team_dir(team_id).join(format!("inbox-{role}.jsonl"))
    }

    fn load_config(&self, team_id: &str) -> Result<TeamConfig, String> {
        let config_path = self.team_dir(team_id).join("team.json");
        let s = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("cannot read team.json: {e}"))?;
        serde_json::from_str(&s).map_err(|e| format!("invalid team.json: {e}"))
    }

    fn append_message(&self, team_id: &str, role: &str, msg: &TeamMessage) -> Result<(), String> {
        let path = self.inbox_path(team_id, role);
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot open inbox for '{role}': {e}"))?;
        writeln!(file, "{}", line).map_err(|e| e.to_string())
    }
}

#[async_trait]
impl Tool for SendMessageTeamTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> &str {
        "Send a message to a team member's inbox. Use 'to': 'all' to broadcast to all members."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": { "type": "string" },
                "from": { "type": "string", "description": "Sender role name" },
                "to": { "type": "string", "description": "Recipient role name or 'all'" },
                "message": { "type": "string" },
                "message_type": {
                    "type": "string",
                    "enum": ["Chat", "TaskAssignment", "StatusUpdate", "Completion", "Error"],
                    "default": "Chat"
                }
            },
            "required": ["team_id", "from", "to", "message"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let team_id = match input["team_id"].as_str() {
            Some(id) => id,
            None => return ToolResult::error("missing 'team_id'"),
        };
        let from = match input["from"].as_str() {
            Some(f) => f,
            None => return ToolResult::error("missing 'from'"),
        };
        let to = match input["to"].as_str() {
            Some(t) => t,
            None => return ToolResult::error("missing 'to'"),
        };
        let content = match input["message"].as_str() {
            Some(m) => m,
            None => return ToolResult::error("missing 'message'"),
        };
        let message_type = parse_message_type(input["message_type"].as_str().unwrap_or("Chat"));

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if to == "all" {
            // Load config to get all member roles
            let config = match self.load_config(team_id) {
                Ok(c) => c,
                Err(e) => return ToolResult::error(e),
            };
            let mut sent_to = Vec::new();
            for member in &config.members {
                let msg = TeamMessage {
                    from: from.to_string(),
                    to: member.role.clone(),
                    content: content.to_string(),
                    timestamp,
                    message_type: message_type.clone(),
                };
                if let Err(e) = self.append_message(team_id, &member.role, &msg) {
                    return ToolResult::error(e);
                }
                sent_to.push(member.role.clone());
            }
            ToolResult::success(
                serde_json::to_string(&json!({ "sent_to": sent_to })).unwrap_or_default(),
            )
        } else {
            let msg = TeamMessage {
                from: from.to_string(),
                to: to.to_string(),
                content: content.to_string(),
                timestamp,
                message_type,
            };
            if let Err(e) = self.append_message(team_id, to, &msg) {
                return ToolResult::error(e);
            }
            ToolResult::success(
                serde_json::to_string(&json!({ "sent_to": to })).unwrap_or_default(),
            )
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

fn parse_message_type(s: &str) -> MessageType {
    match s {
        "TaskAssignment" => MessageType::TaskAssignment,
        "StatusUpdate" => MessageType::StatusUpdate,
        "Completion" => MessageType::Completion,
        "Error" => MessageType::Error,
        _ => MessageType::Chat,
    }
}
