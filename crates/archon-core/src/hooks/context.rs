//! HookContext — enriched context passed to hook executors.
//!
//! Provides a structured snapshot of the current session state when a hook
//! fires, including tool name/input/output, session metadata, and authority.

use serde::{Deserialize, Serialize};

use super::types::{HookEvent, SourceAuthority};

// ---------------------------------------------------------------------------
// HookContext
// ---------------------------------------------------------------------------

/// Full context passed to a hook executor at invocation time.
///
/// Serializes to JSON so command hooks receive it via stdin and HTTP hooks
/// receive it as the request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Which hook event triggered this invocation.
    pub hook_event: HookEvent,
    /// Name of the tool being invoked (if tool-related event).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tool input JSON (for PreToolUse / PostToolUse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    /// Tool output JSON (for PostToolUse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<serde_json::Value>,
    /// Unique session identifier.
    pub session_id: String,
    /// Agent identifier (for sub-agent scoped hooks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// ISO 8601 timestamp of when the hook fired.
    pub timestamp: String,
    /// Current permission mode (e.g. "normal", "plan", "bypass").
    pub permission_mode: String,
    /// Current working directory.
    pub cwd: String,
    /// Name of the previously executed tool (for sequencing logic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_tool: Option<String>,
    /// Conversation turn counter (0-based).
    pub conversation_turn: u32,
    /// Source authority of the hook config that triggered this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_authority: Option<SourceAuthority>,
}

impl HookContext {
    /// Start building a new `HookContext` for the given event.
    pub fn builder(event: HookEvent) -> HookContextBuilder {
        HookContextBuilder::new(event)
    }

    /// Serialize this context to a `serde_json::Value`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("HookContext serialization cannot fail")
    }
}

// ---------------------------------------------------------------------------
// HookContextBuilder
// ---------------------------------------------------------------------------

/// Builder for [`HookContext`] with sensible defaults.
///
/// Required fields (`session_id`, `cwd`) default to empty strings; callers
/// should always set them via the builder methods.
#[derive(Debug, Clone)]
pub struct HookContextBuilder {
    hook_event: HookEvent,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_output: Option<serde_json::Value>,
    session_id: String,
    agent_id: Option<String>,
    timestamp: Option<String>,
    permission_mode: String,
    cwd: String,
    previous_tool: Option<String>,
    conversation_turn: u32,
    source_authority: Option<SourceAuthority>,
}

impl HookContextBuilder {
    /// Create a new builder for the given hook event.
    fn new(event: HookEvent) -> Self {
        Self {
            hook_event: event,
            tool_name: None,
            tool_input: None,
            tool_output: None,
            session_id: String::new(),
            agent_id: None,
            timestamp: None,
            permission_mode: "normal".to_string(),
            cwd: String::new(),
            previous_tool: None,
            conversation_turn: 0,
            source_authority: None,
        }
    }

    /// Set the tool name.
    pub fn tool_name(mut self, name: String) -> Self {
        self.tool_name = Some(name);
        self
    }

    /// Set the tool input JSON.
    pub fn tool_input(mut self, input: serde_json::Value) -> Self {
        self.tool_input = Some(input);
        self
    }

    /// Set the tool output JSON.
    pub fn tool_output(mut self, output: serde_json::Value) -> Self {
        self.tool_output = Some(output);
        self
    }

    /// Set the session identifier.
    pub fn session_id(mut self, id: String) -> Self {
        self.session_id = id;
        self
    }

    /// Set the agent identifier.
    pub fn agent_id(mut self, id: String) -> Self {
        self.agent_id = Some(id);
        self
    }

    /// Override the timestamp (ISO 8601). If not called, `build()` uses
    /// the current UTC time.
    pub fn timestamp(mut self, ts: String) -> Self {
        self.timestamp = Some(ts);
        self
    }

    /// Set the permission mode (default: `"normal"`).
    pub fn permission_mode(mut self, mode: String) -> Self {
        self.permission_mode = mode;
        self
    }

    /// Set the current working directory.
    pub fn cwd(mut self, cwd: String) -> Self {
        self.cwd = cwd;
        self
    }

    /// Set the previously executed tool name.
    pub fn previous_tool(mut self, name: String) -> Self {
        self.previous_tool = Some(name);
        self
    }

    /// Set the conversation turn counter.
    pub fn conversation_turn(mut self, turn: u32) -> Self {
        self.conversation_turn = turn;
        self
    }

    /// Set the source authority.
    pub fn source_authority(mut self, authority: SourceAuthority) -> Self {
        self.source_authority = Some(authority);
        self
    }

    /// Build the [`HookContext`].
    ///
    /// If no timestamp was provided, the current UTC time in RFC 3339 format
    /// is used.
    pub fn build(self) -> HookContext {
        let timestamp = self
            .timestamp
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

        HookContext {
            hook_event: self.hook_event,
            tool_name: self.tool_name,
            tool_input: self.tool_input,
            tool_output: self.tool_output,
            session_id: self.session_id,
            agent_id: self.agent_id,
            timestamp,
            permission_mode: self.permission_mode,
            cwd: self.cwd,
            previous_tool: self.previous_tool,
            conversation_turn: self.conversation_turn,
            source_authority: self.source_authority,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults() {
        let ctx = HookContext::builder(HookEvent::PreToolUse)
            .session_id("s1".into())
            .cwd("/tmp".into())
            .build();
        assert_eq!(ctx.conversation_turn, 0);
        assert_eq!(ctx.permission_mode, "normal");
        assert!(ctx.agent_id.is_none());
    }

    #[test]
    fn to_json_roundtrip() {
        let ctx = HookContext::builder(HookEvent::SessionStart)
            .session_id("s2".into())
            .cwd("/home".into())
            .build();
        let val = ctx.to_json();
        let restored: HookContext =
            serde_json::from_value(val).expect("roundtrip");
        assert_eq!(restored.session_id, "s2");
    }
}
