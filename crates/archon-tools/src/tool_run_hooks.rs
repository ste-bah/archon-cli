//! The hooks a host installs around every tool run: an admission decision
//! before it, and an outcome report after it. Split out of `tool.rs` for size;
//! `crate::tool` re-exports every item, so callers name them there.

use std::sync::Arc;

use crate::tool::PermissionLevel;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolRunAdmissionRequest {
    pub session_id: String,
    pub parent_action_id: String,
    pub tool_use_id: String,
    pub attempt: u32,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub permission_level: PermissionLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRunAdmission {
    Allowed,
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRunAttemptOutcome {
    pub session_id: String,
    pub parent_action_id: String,
    pub tool_use_id: String,
    pub attempt: u32,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub permission_level: PermissionLevel,
    pub blocked: bool,
    pub is_error: bool,
    /// Whether `ToolRunAdmissionCallback` ran for this attempt.
    ///
    /// This callback used to fire only when admission ran — i.e. only for
    /// non-`Safe` tools with an admission callback installed. Ambient topology
    /// tracing needs *every* attempt, so the filter was removed and this flag
    /// took its place.
    ///
    /// **A consumer that correlates against admission state must check this
    /// field.** The world-model guardrail does: it looks up the persisted
    /// admission decision by action id, and for an attempt that was never
    /// admitted there is nothing to find. Before this flag existed the absence
    /// of a decision was inferred from the callback simply not firing.
    pub admission_evaluated: bool,
}

pub type ToolRunAdmissionCallback =
    Arc<dyn Fn(ToolRunAdmissionRequest) -> ToolRunAdmission + Send + Sync>;
pub type ToolRunOutcomeCallback = Arc<dyn Fn(ToolRunAttemptOutcome) + Send + Sync>;
