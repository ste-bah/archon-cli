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
//!
//! "The gate" is more than the permission checker, and for a while this file
//! only honoured that one. A model-issued call reaches the tool through
//! [`ToolRegistry::dispatch`], which layers the ToolRun admission callback, the
//! sandbox capability check, the per-tool time budget and the repeat-tool loop
//! guard on top of `Tool::execute`. Calling `Tool::execute` here ran the same
//! registry with none of them: a tool an operator had blocked was blocked for
//! the model and available to a three-line script. So the call goes through
//! `dispatch` and the permission check above it is the *additional* question a
//! script has to answer, never the only one.

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

// Issue 299: no run totals. Each result is bounded and a run of tool calls
// that brings no new answer pauses the run; see `workflow_script_tools_progress.rs`.
#[path = "workflow_script_tools_progress.rs"]
mod progress;
pub(crate) use progress::ToolCallBudget;

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
    /// The harness's per-call id, carried through to become this attempt's
    /// `tool_run_tool_use_id`.
    ///
    /// Admission and its outcome tap key their records on that id. Left unset
    /// every call in a run would present the same empty identity, so a
    /// topology node id would collide with the previous call's and the
    /// guardrail's outcome rows would overwrite each other — a gate fed one
    /// indistinguishable caller cannot tell a repeat from a first attempt.
    #[serde(default)]
    id: String,
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
    /// Set when `content` was cut to [`progress::MAX_RESULT_BYTES`]; absent otherwise,
    /// so an ordinary result keeps its exact shape.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// The full result's size, present only when it was truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_bytes: Option<usize>,
}

/// The registry and gate a script's tool calls run through.
///
/// Built once per run and shared: `create_default_registry` walks the working
/// tree, and doing that per call would make `tool()` slower than the model
/// round-trip it replaces.
pub(crate) struct ScriptToolHost {
    audited_writes: bool,
    registry: ToolRegistry,
    checker: PermissionChecker,
    /// What every tool call of this run is dispatched with.
    ///
    /// One context for the run rather than one built per call, because what it
    /// carries is run-scoped: the admission callback and its outcome tap are
    /// installed once for a session id, and the repeat-tool chain is keyed by
    /// that same id. Cloned per call only to stamp the attempt's id on it.
    context: ToolContext,
}

impl ScriptToolHost {
    pub(crate) fn require_audited_writes(&mut self) {
        self.audited_writes = true;
    }
    /// Build from the loaded configuration, exactly as a session does.
    pub(crate) fn new(working_dir: std::path::PathBuf, session_id: String) -> WorkflowResult<Self> {
        let config = archon_core::config::load_config().map_err(|error| {
            WorkflowError::SpecInvalid(format!(
                "workflow tool calls need the archon config, which failed to load: {error}"
            ))
        })?;
        // Bound before the literal below moves it: the activity sink is
        // named by the same id the context carries, so the two cannot drift.
        let run_session_id = session_id.clone();
        let mut context = ToolContext {
            working_dir: working_dir.clone(),
            session_id,
            // Plan mode would narrow the registry further; a script is not
            // planning, it is executing an authored orchestration.
            mode: AgentMode::Normal,
            // Same two calls session boot and the workflow CLI path make, from
            // the same loaded config, so a script lands in the world an
            // operator configured rather than always on the host.
            //
            // A script's `Bash` used to run on the host whatever `[sandbox]`
            // said, because the context was built from `ToolContext::default()`
            // and never learned a backend existed. The permission gate still
            // ran, so the call was authorised — and then executed in the wrong
            // world. Permission and confinement are different questions and
            // only the first was asked.
            sandbox: crate::runtime::sandbox_world::isolation_backend(&config.sandbox),
            // Paired with `sandbox` deliberately: the read-before-edit guard
            // and the file tools both read this, and a context carrying one
            // without the other puts them in disagreement about which world
            // they are looking at. A filesystem that cannot be built fails the
            // call, exactly as it fails session boot. Degrading quietly to the
            // host is the failure, not the mitigation.
            fs: archon_core::sandbox::sandbox_filesystem(&config.sandbox, &working_dir).map_err(
                |error| {
                    WorkflowError::SpecInvalid(format!(
                        "workflow tool calls need the sandbox filesystem, which failed to \
                         build: {error}"
                    ))
                },
            )?,
            // Every tool-lifecycle emitter is guarded on this being present,
            // so `None` does not weaken the signal, it removes it: a script's
            // tool calls would pass through `dispatch`, emit ToolStarted and
            // ToolCompleted, and have them land nowhere. The run that most
            // needs a record of what it did is the unattended one.
            activity_sink: crate::session::session_activity_sink(&run_session_id),
            // The guard reads its policy from the CONTEXT, so omitting this
            // takes `RepeatToolConfig::default()` and `[guard.repeat_tool]`
            // applies to every caller except this one. Same reasoning, and the
            // same line, as `workflow_tool_context` in `pipeline_support.rs`.
            repeat_tool: config.guard.repeat_tool.clone(),
            ..ToolContext::default()
        };
        // The blocked-tool gate, installed by the one function every other
        // host path installs it with. Routing through `dispatch` without this
        // would compile, run, and consult a callback that is always `None` —
        // the gate present and inert, which is worse than absent because it
        // reads as closed.
        crate::command::world_model::configure_tool_run_context(&config, &mut context);
        Ok(Self {
            audited_writes: false,
            registry: archon_core::dispatch::create_default_registry(working_dir, None),
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
            context,
        })
    }

    /// Run one tool call on behalf of a script.
    ///
    /// `call_id` is the harness's id for this call and becomes the attempt's
    /// tool-use id, which is what lets admission tell two calls apart.
    pub(crate) async fn run(
        &self,
        call_id: &str,
        request: &RunToolRequest,
    ) -> Result<RunToolResponse, String> {
        let Some(tool) = self.registry.lookup(&request.name) else {
            return Err(format!(
                "no tool named {:?}. Workflow scripts reach the same registry an agent does.",
                request.name
            ));
        };

        if self.audited_writes && tool.working_tree_effect().requires_filesystem_observation() {
            return Err(format!(
                "{} can mutate repository files; use an audited write call with declared targets so its patch passes manifest checks",
                request.name
            ));
        }
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

        // `dispatch` and not `tool.execute`: it is the same public door the
        // subagent path goes through, and everything between it and the tool —
        // admission, the sandbox capability check, the per-tool time budget,
        // the repeat-tool chain — is what a script was skipping. The lookup
        // above stays because the permission check needs the description and
        // because the not-found message is written for a script author, not a
        // model.
        let result = self
            .registry
            .dispatch(
                &request.name,
                request.input.clone(),
                &self.context.with_tool_run_attempt(call_id, 0),
            )
            .await;
        Ok(RunToolResponse {
            tool: request.name.clone(),
            content: result.content,
            is_error: result.is_error,
            truncated: false,
            original_bytes: None,
        })
    }
}

fn lock_budget(
    budget: &std::sync::Mutex<ToolCallBudget>,
) -> std::sync::MutexGuard<'_, ToolCallBudget> {
    budget
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Parse, run and serialise one `runTool` host call.
///
/// A stall comes back as `ControlPaused` with its evidence held on `budget`
/// ([`ToolCallBudget::take_stall`]) for the host to record with the pause.
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

    // Compared exactly, never previewed: two calls are the same call only if
    // every argument is the same.
    let arguments = serde_json::to_string(&request.input).unwrap_or_default();
    // Checked BEFORE the call runs. A stall is only knowable from calls that
    // already answered, so the call that would repeat a full streak is refused
    // unexecuted and the run pauses: nothing runs and is then thrown away.
    if let Some(message) =
        lock_budget(budget).refuse_repeat(&envelope.id, &request.name, &arguments)
    {
        return Err(WorkflowError::ControlPaused(message));
    }

    let response = match host.run(&envelope.id, &request).await {
        Ok(response) => response,
        // A refusal is the script's to handle — it may have a fallback — so it
        // comes back as a failed result rather than killing the run. The
        // message is the one a model-issued call would have produced.
        Err(message) => RunToolResponse {
            tool: request.name.clone(),
            content: message,
            is_error: true,
            truncated: false,
            original_bytes: None,
        },
    };
    // The call has run once; an oversized answer is cut and marked, never
    // refused, so its effect and the script's view of it agree.
    let response = progress::bound_response(response);
    lock_budget(budget).record(&envelope.id, &request.name, &arguments, &response);

    serde_json::to_string(&response).map_err(|error| {
        WorkflowError::SpecInvalid(format!("could not serialise a tool result: {error}"))
    })
}

#[cfg(test)]
#[path = "workflow_script_tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_script_tools_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "workflow_script_tools_progress_tests.rs"]
mod progress_tests;
