//! What `archon sandbox explain --tool <name>` says about a single tool.
//!
//! None of the answer is written down here. A tool's class is declared on the
//! tool itself (#201 Phase 3), which tools a mode defers to the backend is
//! [`SandboxRouteMode`], and the decision on a class is
//! [`check_capability`] — the one gate docker, ssh and openshell all call. This
//! file resolves the name against the registry the runtime builds and then asks
//! those two. A copy of any of it here would be a fourth allowlist drifting
//! away from the decision the backends actually make, which is the failure
//! #201 exists to end.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_core::sandbox::{SandboxPolicy, check_capability};
use archon_permissions::{SandboxBackend, ToolCapability};

use crate::runtime::sandbox_mode::{ModeRouting, SandboxRouteMode};

pub(super) fn append_tool_explain(
    output: &mut String,
    policy: &SandboxPolicy,
    tool: Option<&str>,
    command: Option<&str>,
) {
    let Some(tool) = tool.map(str::trim).filter(|tool| !tool.is_empty()) else {
        return;
    };
    output.push_str(&format!("Tool explain\nTool: {tool}\n"));
    match declared_capability(tool) {
        Some(capability) => {
            let (decision, reason) = decide(policy, tool, capability);
            output.push_str(&format!(
                "Capability: {}\nDecision: {decision}\nReason: {reason}\n",
                capability.label()
            ));
            if policy.backend.is_real_isolation() {
                output.push_str(
                    "Readiness: this is the routing decision only; whether the backend is enabled \
                     and reachable is what `archon sandbox doctor` answers\n",
                );
            }
        }
        // Guessing here is the one thing this command must not do: a plausible
        // default for a name nothing declares is an answer about a tool that
        // does not exist.
        None => output.push_str(&format!(
            "Decision: unknown_tool\nReason: no tool named {tool} is registered, so it declares no \
             capability class and there is no decision to report; MCP and plugin tools are \
             registered per session and are not visible from the CLI\n"
        )),
    }
    if let Some(command) = command.map(str::trim).filter(|value| !value.is_empty()) {
        output.push_str(&format!("Command preview: {}\n", command_preview(command)));
    }
}

fn decide(
    policy: &SandboxPolicy,
    tool: &str,
    capability: ToolCapability,
) -> (&'static str, String) {
    if !policy.backend.is_real_isolation() {
        return host_session_decision(tool, capability);
    }
    match SandboxRouteMode::from_config(&policy.mode).route(tool) {
        ModeRouting::Backend => match check_capability(policy.backend.as_str(), tool, capability) {
            Ok(()) => (
                "route_to_sandbox",
                format!(
                    "the {} backend has a seam for {} work, so the call is served in the \
                     sandbox's world rather than on the host",
                    policy.backend,
                    capability.label()
                ),
            ),
            Err(denial) => ("blocked_by_backend", denial),
        },
        ModeRouting::RefusedByMode(reason) => ("blocked_by_sandbox_mode", reason.to_string()),
        // `Ok(())` from the wrapper, but the backend never saw the call, and
        // saying "routed" here would be the drift this file is being fixed for.
        ModeRouting::PermissionPreflight => (
            "permission_preflight_only",
            format!(
                "sandbox.mode = {} defers only shell execution to the backend, so {tool} is \
                 decided by the normal permission preflight rather than by the sandbox; the mode \
                 scopes which decisions the backend makes, not where work lands — under every \
                 mode the file tools use the backend's filesystem and a terminal opens in its \
                 world",
                policy.mode
            ),
        ),
    }
}

/// `disabled` and `logical` both leave the session on the host: neither builds
/// an isolation backend, so `native_session_sandbox_backend` installs the
/// `/sandbox` toggle instead, and it starts off. What the toggle does when it
/// is on is the toggle's own answer, asked here rather than restated.
fn host_session_decision(tool: &str, capability: ToolCapability) -> (&'static str, String) {
    let toggled_on =
        archon_tui::sandbox::SharedSandboxFlag::with_flag(Arc::new(AtomicBool::new(true)));
    let when_on = match toggled_on.check(tool, capability, &serde_json::Value::Null) {
        Ok(()) => "`/sandbox on` would still allow it".to_string(),
        Err(denial) => format!("`/sandbox on` would refuse it — {denial}"),
    };
    (
        "permission_preflight_only",
        format!(
            "no isolation backend is configured, so nothing relocates this call and the normal \
             permission preflight decides it; the session's /sandbox toggle is the only sandbox \
             gate and it starts off, and {when_on}"
        ),
    )
}

/// A tool's class lives on the tool and nowhere else, so the only honest route
/// from a CLI string to one is to build the registry the runtime builds and ask
/// it. It is not free — every built-in tool is constructed — but explain runs
/// once per invocation and constructs no sessions, while the cheap alternative
/// is a name table here that nothing keeps in step with the tools. That table
/// is the defect being removed.
fn declared_capability(tool: &str) -> Option<ToolCapability> {
    let registry = archon_core::dispatch::create_default_registry(
        std::env::current_dir().unwrap_or_default(),
        None,
    );
    registry.get(tool).map(|tool| tool.capability())
}

fn command_preview(command: &str) -> String {
    let compact = command.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 120;
    if compact.chars().count() <= MAX {
        compact
    } else {
        let mut preview = compact.chars().take(MAX).collect::<String>();
        preview.push_str("...");
        preview
    }
}
