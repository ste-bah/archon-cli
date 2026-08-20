//! Agent-activity payload types, split out of `events.rs` for the 500-line
//! file-size gate.
//!
//! These describe what a subagent is doing; `events.rs` keeps the event enum
//! that carries them. Re-exported from `events` so every existing
//! `crate::events::AgentActivityUpdate` path still resolves.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivityRole {
    Parent,
    Subagent,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivityStatus {
    Queued,
    Running,
    Waiting,
    WaitingForTool,
    Backgrounded,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentActivityUpdate {
    pub id: String,
    pub name: String,
    pub role: AgentActivityRole,
    pub status: AgentActivityStatus,
    pub current_tool: Option<String>,
    pub detail: Option<String>,
    pub run_id: Option<String>,
    pub parent_id: Option<String>,
    pub artifact_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
}

pub const ACTIVITY_STREAM_PREFIX: &str = "archon_activity_stream:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityStreamLineKind {
    Status,
    Thinking,
    Text,
    ToolCall,
    ToolResult,
    FinalOutput,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityStreamUpdate {
    pub id: String,
    pub name: String,
    pub role: AgentActivityRole,
    pub status: AgentActivityStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub kind: ActivityStreamLineKind,
    pub text: String,
    pub tool: Option<String>,
    pub is_error: bool,
}

impl ActivityStreamUpdate {
    pub fn from_activity_event(event: archon_observability::AgentActivityEvent) -> Self {
        let base = AgentActivityUpdate::from(event.clone());
        if let Some(payload) = activity_stream_payload(&event.message) {
            return Self {
                id: base.id,
                name: base.name,
                role: base.role,
                status: base.status,
                provider: base.provider,
                model: base.model,
                kind: payload.kind,
                text: payload.text,
                tool: payload.tool,
                is_error: payload.is_error,
            };
        }
        Self {
            id: base.id,
            name: base.name,
            role: base.role,
            status: base.status,
            provider: base.provider,
            model: base.model,
            kind: ActivityStreamLineKind::Status,
            text: base
                .detail
                .unwrap_or_else(|| format!("{:?}", base.status).to_lowercase()),
            tool: base.current_tool,
            is_error: matches!(
                base.status,
                AgentActivityStatus::Failed | AgentActivityStatus::Cancelled
            ),
        }
    }
}

pub fn is_activity_stream_payload(message: &str) -> bool {
    message.starts_with(ACTIVITY_STREAM_PREFIX)
}

struct ActivityPayload {
    kind: ActivityStreamLineKind,
    text: String,
    tool: Option<String>,
    is_error: bool,
}

fn activity_stream_payload(message: &str) -> Option<ActivityPayload> {
    let raw = message.strip_prefix(ACTIVITY_STREAM_PREFIX)?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let kind = match value.get("kind")?.as_str()? {
        "thinking" => ActivityStreamLineKind::Thinking,
        "text" => ActivityStreamLineKind::Text,
        "tool_call" => ActivityStreamLineKind::ToolCall,
        "tool_result" => ActivityStreamLineKind::ToolResult,
        "final" => ActivityStreamLineKind::FinalOutput,
        "error" => ActivityStreamLineKind::Error,
        _ => ActivityStreamLineKind::Status,
    };
    Some(ActivityPayload {
        kind,
        text: value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        tool: value
            .get("tool")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        is_error: value
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

impl From<archon_observability::AgentActivityEvent> for AgentActivityUpdate {
    fn from(event: archon_observability::AgentActivityEvent) -> Self {
        let role = activity_role(&event);
        let status = activity_status(&event);
        let id = activity_id(&event);
        let name = activity_name(&event, role);
        let current_tool = match event.kind {
            archon_observability::AgentActivityKind::ToolStarted
            | archon_observability::AgentActivityKind::ToolCompleted
            | archon_observability::AgentActivityKind::ToolFailed => Some(event.message.clone()),
            _ => None,
        };

        Self {
            id,
            name,
            role,
            status,
            current_tool,
            detail: Some(event.message),
            run_id: event.run_id,
            parent_id: event.parent_id,
            artifact_id: event.artifact_id,
            provider: event.provider,
            model: event.model,
            cost_usd: event.cost_usd,
        }
    }
}

fn activity_role(event: &archon_observability::AgentActivityEvent) -> AgentActivityRole {
    match event.kind {
        archon_observability::AgentActivityKind::ParentTurnStarted
        | archon_observability::AgentActivityKind::ParentTurnCompleted
        | archon_observability::AgentActivityKind::ToolStarted
        | archon_observability::AgentActivityKind::ToolCompleted
        | archon_observability::AgentActivityKind::ToolFailed => AgentActivityRole::Parent,
        archon_observability::AgentActivityKind::AgentBackgrounded
        | archon_observability::AgentActivityKind::AgentResumed => AgentActivityRole::Background,
        _ => AgentActivityRole::Subagent,
    }
}

fn activity_status(event: &archon_observability::AgentActivityEvent) -> AgentActivityStatus {
    match event.kind {
        archon_observability::AgentActivityKind::ToolStarted => {
            return AgentActivityStatus::WaitingForTool;
        }
        archon_observability::AgentActivityKind::ToolCompleted => {
            return AgentActivityStatus::Complete;
        }
        archon_observability::AgentActivityKind::ToolFailed => {
            return AgentActivityStatus::Failed;
        }
        _ => {}
    }

    match event.status {
        archon_observability::AgentActivityStatus::Queued => AgentActivityStatus::Queued,
        archon_observability::AgentActivityStatus::Running => AgentActivityStatus::Running,
        archon_observability::AgentActivityStatus::Waiting => AgentActivityStatus::Waiting,
        archon_observability::AgentActivityStatus::Backgrounded => {
            AgentActivityStatus::Backgrounded
        }
        archon_observability::AgentActivityStatus::Completed => AgentActivityStatus::Complete,
        archon_observability::AgentActivityStatus::Failed => AgentActivityStatus::Failed,
        archon_observability::AgentActivityStatus::Cancelled => AgentActivityStatus::Cancelled,
    }
}

fn activity_id(event: &archon_observability::AgentActivityEvent) -> String {
    event
        .subagent_id
        .clone()
        .or_else(|| event.agent_id.clone())
        .or_else(|| event.run_id.clone())
        .unwrap_or_else(|| "parent".to_string())
}

fn activity_name(
    event: &archon_observability::AgentActivityEvent,
    role: AgentActivityRole,
) -> String {
    event
        .agent_key
        .clone()
        .or_else(|| event.subagent_type.clone())
        .or_else(|| event.model.clone())
        .unwrap_or_else(|| match role {
            AgentActivityRole::Parent => "Parent".to_string(),
            AgentActivityRole::Subagent => "Subagent".to_string(),
            AgentActivityRole::Background => "Background".to_string(),
        })
}
