//! Real tool calls from inside a workflow script (#189 Phase 4).
//!
//! The script runtime already had `w.tool(...)`, but only for three
//! workflow-internal pseudo-tools — `checkpoint`, `saveArtifact` and
//! `requireArtifact`. A script could not read a file, so every ordinary file
//! operation inside an orchestration cost a whole model round-trip: spawn an
//! agent, pay for a turn, and get back what `Read` would have returned.
//!
//! `runTool` is the seam for the real registry. It is a separate host method
//! rather than an extension of `w.tool` because the two mean different things,
//! and because widening the existing allowlist would have changed what an
//! already-authored script does.
//!
//! Running in the host is not a licence to skip the gate. Every call goes
//! through the same [`PermissionChecker`] a model-issued call goes through, and
//! a script must not become a way to run what a model would have been stopped
//! from running.

use std::sync::Arc;

use archon_core::dispatch::ToolRegistry;
use std::str::FromStr;

use archon_permissions::checker::PermissionChecker;
use archon_permissions::mode::PermissionDecision;
use archon_tools::tool::{AgentMode, ToolContext};
use archon_workflow::{WorkflowError, WorkflowResult};

/// Host method name. Deliberately not a `WorkflowV2HostMethod` variant: that
/// enum is matched exhaustively in dozens of places that have nothing to say
/// about a tool call, and the bridge dispatches on the raw string anyway.
pub(crate) const RUN_TOOL_METHOD: &str = "runTool";

/// Most tool calls one script run may make.
///
/// A script is a loop with no model in it to get bored, so an accidental
/// unbounded loop over a directory would otherwise run until the watchdog
/// fires. High enough that no plausible orchestration reaches it.
pub(crate) const MAX_TOOL_CALLS: usize = 500;

/// Most bytes all tool calls in one run may return in total.
///
/// The per-call cap is the tool's own; this bounds the sum, because the failure
/// worth preventing is a thousand small reads rather than one large one.
pub(crate) const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

/// What the script asked for.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct RunToolRequest {
    /// Registry name, e.g. `Read`.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// The envelope every host call arrives in: `{ id, options }`.
///
/// Shared with agent calls rather than given a shape of its own, so the
/// pending-call tracking and the id-per-call rule in the harness apply here
/// unchanged.
#[derive(Debug, Clone, serde::Deserialize)]
struct RunToolEnvelope {
    #[serde(default)]
    options: RunToolRequest,
}

/// What it gets back — the shape of a `ToolResult`, plus the name for a script
/// that logs what it did.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RunToolResponse {
    pub tool: String,
    pub content: String,
    pub is_error: bool,
}

/// Running totals for one script run.
#[derive(Debug, Default)]
pub(crate) struct ToolCallBudget {
    pub calls: usize,
    pub bytes: usize,
}

impl ToolCallBudget {
    /// Charge one result against the budget, or explain what was exceeded.
    ///
    /// Charged after the call rather than before: the byte cost is not known
    /// until the tool has answered, and refusing the call that crosses the line
    /// while still returning its output would spend the bytes and lose them.
    pub(crate) fn admit(&mut self, bytes: usize) -> Result<(), String> {
        if self.calls >= MAX_TOOL_CALLS {
            return Err(format!(
                "this script has made {MAX_TOOL_CALLS} tool calls, which is the limit for one run"
            ));
        }
        if self.bytes.saturating_add(bytes) > MAX_TOTAL_BYTES {
            return Err(format!(
                "this script's tool calls have returned {} bytes, and the limit for one run is {MAX_TOTAL_BYTES}",
                self.bytes
            ));
        }
        self.calls += 1;
        self.bytes += bytes;
        Ok(())
    }
}

/// The registry and gate a script's tool calls run through.
///
/// Built once per run and shared: `create_default_registry` walks the working
/// tree, and doing that per call would make `tool()` slower than the model
/// round-trip it replaces.
pub(crate) struct ScriptToolHost {
    registry: ToolRegistry,
    checker: PermissionChecker,
    working_dir: std::path::PathBuf,
    session_id: String,
}

impl ScriptToolHost {
    /// Build from the loaded configuration, exactly as a session does.
    pub(crate) fn new(working_dir: std::path::PathBuf, session_id: String) -> WorkflowResult<Self> {
        let config = archon_core::config::load_config().map_err(|error| {
            WorkflowError::SpecInvalid(format!(
                "workflow tool calls need the archon config, which failed to load: {error}"
            ))
        })?;
        Ok(Self {
            registry: archon_core::dispatch::create_default_registry(working_dir.clone(), None),
            checker: PermissionChecker::new(
                // Same parse the session does, and the same fallback: an
                // unrecognised mode string must not silently become the most
                // permissive one.
                archon_permissions::mode::PermissionMode::from_str(&config.permissions.mode)
                    .unwrap_or_default(),
                archon_permissions::rules::RuleSet {
                    always_allow: config.permissions.always_allow.clone(),
                    always_deny: config.permissions.always_deny.clone(),
                    always_ask: config.permissions.always_ask.clone(),
                },
            ),
            working_dir,
            session_id,
        })
    }

    /// Run one tool call on behalf of a script.
    pub(crate) async fn run(&self, request: &RunToolRequest) -> Result<RunToolResponse, String> {
        let Some(tool) = self.registry.lookup(&request.name) else {
            return Err(format!(
                "no tool named {:?}. Workflow scripts reach the same registry an agent does.",
                request.name
            ));
        };

        let arguments = serde_json::to_string(&request.input).unwrap_or_else(|_| "{}".to_string());
        match self
            .checker
            .check(&request.name, tool.description(), &arguments)
        {
            PermissionDecision::Allow => {}
            // There is nobody to ask. A script runs unattended, so a decision
            // that means "confirm with the user" can only be a refusal here —
            // and saying so beats hanging, or worse, quietly allowing.
            PermissionDecision::NeedsPermission(reason) => {
                return Err(format!(
                    "{} needs permission ({reason}), and a workflow script runs with nobody to \
                     ask. Allow it in config under [permissions] always_allow, or do this work \
                     in an agent call.",
                    request.name
                ));
            }
            PermissionDecision::Deny(reason) => {
                return Err(format!("{} is denied: {reason}", request.name));
            }
        }

        let context = ToolContext {
            working_dir: self.working_dir.clone(),
            session_id: self.session_id.clone(),
            // Plan mode would narrow the registry further; a script is not
            // planning, it is executing an authored orchestration.
            mode: AgentMode::Normal,
            ..ToolContext::default()
        };
        let result = tool.execute(request.input.clone(), &context).await;
        Ok(RunToolResponse {
            tool: request.name.clone(),
            content: result.content,
            is_error: result.is_error,
        })
    }
}

/// Parse, run and serialise one `runTool` host call.
pub(crate) async fn execute_run_tool(
    host: &Arc<ScriptToolHost>,
    budget: &Arc<std::sync::Mutex<ToolCallBudget>>,
    payload: &str,
) -> WorkflowResult<String> {
    let envelope: RunToolEnvelope = serde_json::from_str(payload).map_err(|error| {
        WorkflowError::SpecInvalid(format!(
            "tool() was called with an unreadable request: {error}"
        ))
    })?;
    let request = envelope.options;
    if request.name.trim().is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "tool() was called without a tool name".to_string(),
        ));
    }

    let response = match host.run(&request).await {
        Ok(response) => response,
        // A refusal is the script's to handle — it may have a fallback — so it
        // comes back as a failed result rather than killing the run. The
        // message is the one a model-issued call would have produced.
        Err(message) => RunToolResponse {
            tool: request.name.clone(),
            content: message,
            is_error: true,
        },
    };

    {
        let mut budget = budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Exceeding the run budget *is* fatal: unlike a refused call, there is
        // no state in which continuing produces a smaller total.
        budget
            .admit(response.content.len())
            .map_err(WorkflowError::SpecInvalid)?;
    }

    serde_json::to_string(&response).map_err(|error| {
        WorkflowError::SpecInvalid(format!("could not serialise a tool result: {error}"))
    })
}

#[cfg(test)]
#[path = "workflow_script_tools_tests.rs"]
mod tests;
