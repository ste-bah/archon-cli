use super::{
    ElicitationAction, HookOutcome, HookResult, PermissionBehavior, PermissionUpdate,
    SourceAuthority,
};

// ---------------------------------------------------------------------------
// AggregatedHookResult — accumulated result from ALL matching hooks
// Reference: Claude Code types/hooks.ts:277-290
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct AggregatedHookResult {
    pub blocking_errors: Vec<String>,
    /// Failures that allow execution to continue, including no-progress stops.
    /// These are logged where they occur; the runtime does not show them.
    pub nonblocking_errors: Vec<String>,
    /// The subset of `nonblocking_errors` that are allowing no-progress stops.
    /// `PostToolUse` appends these, and only these, to the tool result.
    pub no_progress_stops: Vec<String>,
    pub additional_contexts: Vec<String>,
    pub updated_input: Option<serde_json::Value>,
    pub updated_mcp_tool_output: Option<serde_json::Value>,
    pub permission_behavior: Option<PermissionBehavior>,
    pub permission_decision_reason: Option<String>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
    pub retry: bool,
    pub system_messages: Vec<String>,
    pub status_messages: Vec<String>,
    pub updated_permissions: Vec<PermissionUpdate>,
    /// Collected watch paths from all hook results (REQ-HOOK-017).
    pub watch_paths: Vec<String>,
    /// Elicitation auto-respond action — last writer wins (REQ-HOOK-019).
    pub elicitation_action: Option<ElicitationAction>,
    /// Elicitation content payload — last writer wins (REQ-HOOK-019).
    pub elicitation_content: Option<serde_json::Value>,
}

impl AggregatedHookResult {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge a single HookResult into this aggregate (REQ-HOOK-011).
    pub fn merge(&mut self, result: HookResult) {
        self.merge_harness_result(result, None);
    }

    /// Merge a result together with stop metadata produced by the harness.
    pub(crate) fn merge_harness_result(
        &mut self,
        result: HookResult,
        no_progress_stop: Option<&str>,
    ) {
        match result.outcome {
            HookOutcome::Blocking => self.blocking_errors.push(
                result
                    .reason
                    .clone()
                    .unwrap_or_else(|| "hook blocked (no reason given)".to_owned()),
            ),
            HookOutcome::NonBlockingError => {
                let reason = result
                    .reason
                    .clone()
                    .unwrap_or_else(|| "hook failed (no reason given)".to_owned());
                if no_progress_stop.is_some() {
                    self.no_progress_stops.push(reason.clone());
                }
                self.nonblocking_errors.push(reason);
            }
            HookOutcome::Success | HookOutcome::Cancelled => {}
        }

        // updated_input: last writer wins
        if result.updated_input.is_some() {
            self.updated_input = result.updated_input;
        }

        // updated_mcp_tool_output: last writer wins
        if result.updated_mcp_tool_output.is_some() {
            self.updated_mcp_tool_output = result.updated_mcp_tool_output;
        }

        // additional_context: collect all
        if let Some(ctx) = result.additional_context {
            self.additional_contexts.push(ctx);
        }

        // permission_behavior: policy wins; non-policy cannot Allow blocked tools (REQ-HOOK-004a)
        if let Some(ref pb) = result.permission_behavior
            && *pb != PermissionBehavior::Passthrough
        {
            let is_policy = result.source_authority == Some(SourceAuthority::Policy);
            match pb {
                PermissionBehavior::Allow if !is_policy => {
                    // Non-policy hook cannot grant Allow — silently dropped
                    tracing::warn!("non-policy hook attempted permission_behavior=allow; dropped");
                }
                _ => {
                    self.permission_behavior = Some(pb.clone());
                    if result.permission_decision_reason.is_some() {
                        self.permission_decision_reason = result.permission_decision_reason.clone();
                    }
                }
            }
        }

        // prevent_continuation: any true wins
        if result.prevent_continuation == Some(true) {
            self.prevent_continuation = true;
            if result.stop_reason.is_some() {
                self.stop_reason = result.stop_reason;
            }
        }

        // retry: any true wins
        if result.retry == Some(true) {
            self.retry = true;
        }

        // system_message: collect all
        if let Some(msg) = result.system_message {
            self.system_messages.push(msg);
        }

        // status_message: collect all
        if let Some(msg) = result.status_message {
            self.status_messages.push(msg);
        }

        // updated_permissions: collect all (REQ-HOOK-016)
        if !result.updated_permissions.is_empty() {
            self.updated_permissions.extend(result.updated_permissions);
        }

        // watch_paths: collect all (REQ-HOOK-017)
        if !result.watch_paths.is_empty() {
            self.watch_paths.extend(result.watch_paths);
        }

        // elicitation_action: last writer wins (REQ-HOOK-019)
        if result.elicitation_action.is_some() {
            self.elicitation_action = result.elicitation_action;
        }
        // elicitation_content: last writer wins (REQ-HOOK-019)
        if result.elicitation_content.is_some() {
            self.elicitation_content = result.elicitation_content;
        }
    }

    /// Check if any hook blocked execution.
    pub fn is_blocked(&self) -> bool {
        !self.blocking_errors.is_empty()
    }

    /// Get combined block reason string.
    pub fn block_reason(&self) -> Option<String> {
        if self.blocking_errors.is_empty() {
            None
        } else {
            Some(self.blocking_errors.join("; "))
        }
    }
}
