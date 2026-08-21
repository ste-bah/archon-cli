//! What `archon sandbox explain --tool <name>` says about a single tool.
//!
//! None of the answer is written down here. A tool's class is declared on the
//! tool itself (#201 Phase 3), which tools a mode defers to the backend is
//! [`SandboxRouteMode`], the decision on a class is [`check_capability`] — the
//! one gate docker, ssh and openshell all call — and whether a backend has a
//! session to put a shell in is `SandboxBackend::terminal`, which is asked
//! rather than guessed. This file resolves the name against the registry the
//! runtime builds and then relays those answers, including their wording. A
//! copy of any of it here would be a fourth allowlist drifting away from the
//! decision the backends actually make, which is the failure #201 exists to
//! end.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_core::sandbox::{SandboxConfig, SandboxPolicy, check_capability};
use archon_permissions::sandbox::{SandboxTerminal, SandboxTerminalRequest};
use archon_permissions::{SandboxBackend, ToolCapability, WorldReach};

use crate::runtime::sandbox_mode::{ModeRouting, SandboxRouteMode};

/// The gate is asked about the class, not about the backend's state, so an
/// allow is true of a *ready* backend. Docker being disabled or an ssh host
/// being unreachable is a different question, and this command does not answer
/// it — `doctor` does, and probing here would make `explain` spawn processes.
const READINESS_UNCHECKED: &str = "Readiness: the gate decides the class, not whether the backend is enabled and reachable; \
     `archon sandbox doctor` answers that";

/// Said only where the terminal seam was actually consulted, because that
/// answer *does* include readiness — and for ssh, buying it costs a local
/// version probe.
const TERMINAL_CONSULTED: &str = "Consulted: `SandboxBackend::terminal` on the configured backend, the only thing that knows \
     whether it has a session to attach a TTY to; that answer includes the backend's readiness \
     check, which for some backends spawns a local version probe";

/// `effective` is the configuration after any `--backend` override, so the
/// backend built from it is the one this explanation is about.
pub(super) fn append_tool_explain(
    output: &mut String,
    effective: &SandboxConfig,
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
            let decided = decide(effective, policy, tool, capability);
            output.push_str(&format!(
                "Capability: {}\nDecision: {}\nReason: {}\n",
                capability.label(),
                decided.decision,
                decided.reason
            ));
            if let Some(provenance) = decided.provenance {
                output.push_str(provenance);
                output.push('\n');
            }
        }
        None => output.push_str(&unknown_tool_report(tool)),
    }
    if let Some(command) = command.map(str::trim).filter(|value| !value.is_empty()) {
        output.push_str(&format!("Command preview: {}\n", command_preview(command)));
    }
}

struct ToolDecision {
    decision: &'static str,
    reason: String,
    /// What this answer did or did not consult, when that changes what the
    /// decision means. A decision line that reads as unconditional and is not
    /// is the defect this field exists to prevent.
    provenance: Option<&'static str>,
}

fn decide(
    effective: &SandboxConfig,
    policy: &SandboxPolicy,
    tool: &str,
    capability: ToolCapability,
) -> ToolDecision {
    if !policy.backend.is_real_isolation() {
        return host_session_decision(tool, capability);
    }
    // Asked before the mode, because `terminal()` is delegated under every mode
    // — `ModeScopedSandboxBackend` scopes `check`, never `terminal`. The mode
    // still decides whose call the *permission* is, which is the second half of
    // the reason below.
    if matches!(capability, ToolCapability::WorldBound(WorldReach::Terminal)) {
        return terminal_decision(effective, policy, tool, capability);
    }
    match SandboxRouteMode::from_config(&policy.mode).route(tool) {
        ModeRouting::Backend => match check_capability(policy.backend.as_str(), tool, capability) {
            Ok(allowance) => ToolDecision {
                decision: "route_to_sandbox_when_backend_ready",
                reason: allowance.reason().to_string(),
                provenance: Some(READINESS_UNCHECKED),
            },
            // A denial holds whatever the backend's state is: a host handle is
            // a host handle on a running container too.
            Err(denial) => ToolDecision {
                decision: "blocked_by_backend",
                reason: denial,
                provenance: None,
            },
        },
        ModeRouting::RefusedByMode(reason) => ToolDecision {
            decision: "blocked_by_sandbox_mode",
            reason: reason.to_string(),
            provenance: None,
        },
        // `Ok(())` from the wrapper, but the backend never saw the call, and
        // saying "routed" here would be the drift this file is being fixed for.
        ModeRouting::PermissionPreflight => ToolDecision {
            decision: "permission_preflight_only",
            reason: preflight_reason(policy, tool),
            provenance: None,
        },
    }
}

fn preflight_reason(policy: &SandboxPolicy, tool: &str) -> String {
    format!(
        "sandbox.mode = {} defers only shell execution to the backend, so {tool} is decided by the \
         normal permission preflight rather than by the sandbox; the mode scopes which decisions \
         the backend makes, not where work lands — `ToolContext::fs` and `terminal()` go to the \
         configured backend under every mode",
        policy.mode
    )
}

/// The gate deliberately does not decide a terminal, and says so; asking it and
/// stopping there reported openshell as routing a shell into a world it has no
/// session in. So the backend is built and asked, which is what a session does
/// when the model calls the tool.
fn terminal_decision(
    effective: &SandboxConfig,
    policy: &SandboxPolicy,
    tool: &str,
    capability: ToolCapability,
) -> ToolDecision {
    if let Err(denial) = check_capability(policy.backend.as_str(), tool, capability) {
        return ToolDecision {
            decision: "blocked_by_backend",
            reason: denial,
            provenance: None,
        };
    }
    let Some(backend) = crate::runtime::sandbox_world::isolation_backend(effective) else {
        // `is_real_isolation` was already true, so this is a configuration the
        // constructor rejected rather than a host case. Reporting a route here
        // would be inventing one.
        return ToolDecision {
            decision: "backend_unavailable",
            reason: format!(
                "sandbox.backend = {} names an isolation backend that could not be constructed \
                 from this configuration, so there is no terminal seam to ask",
                policy.backend
            ),
            provenance: None,
        };
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let opened = backend.terminal(&SandboxTerminalRequest {
        shell: None,
        workspace: cwd.clone(),
        cwd,
    });
    let mode_note = terminal_mode_note(policy, tool);
    match opened {
        SandboxTerminal::Open(command) => ToolDecision {
            decision: "route_to_sandbox",
            reason: format!(
                "the backend opens a shell in its own world — {} running at {}; {mode_note}",
                command.shell, command.location
            ),
            provenance: Some(TERMINAL_CONSULTED),
        },
        SandboxTerminal::Refused(refusal) => ToolDecision {
            decision: "refused_by_backend_terminal",
            reason: format!("{refusal}; {mode_note}"),
            provenance: Some(TERMINAL_CONSULTED),
        },
        // Only a backend that does not relocate anything answers this, and
        // `is_real_isolation` excluded those — but the variant is real, and a
        // terminal on the host under an isolating configuration is exactly the
        // thing that must not be reported as a route.
        SandboxTerminal::Host => ToolDecision {
            decision: "runs_on_the_host",
            reason: format!(
                "the backend hands this terminal to the host rather than relocating it; {mode_note}"
            ),
            provenance: Some(TERMINAL_CONSULTED),
        },
    }
}

fn terminal_mode_note(policy: &SandboxPolicy, tool: &str) -> String {
    match SandboxRouteMode::from_config(&policy.mode).route(tool) {
        ModeRouting::Backend => format!(
            "sandbox.mode = {} also defers this tool's permission to the backend",
            policy.mode
        ),
        _ => format!(
            "sandbox.mode = {} leaves this tool's permission to the normal preflight, but the \
             shell itself is the backend's under every mode",
            policy.mode
        ),
    }
}

/// `disabled` and `logical` both leave the session on the host: neither builds
/// an isolation backend, so `native_session_sandbox_backend` installs the
/// `/sandbox` toggle instead, and it starts off. What the toggle does when it
/// is on is the toggle's own answer, asked here rather than restated.
fn host_session_decision(tool: &str, capability: ToolCapability) -> ToolDecision {
    let toggled_on =
        archon_tui::sandbox::SharedSandboxFlag::with_flag(Arc::new(AtomicBool::new(true)));
    let when_on = match toggled_on.check(tool, capability, &serde_json::Value::Null) {
        Ok(()) => "`/sandbox on` would still allow it".to_string(),
        Err(denial) => format!("`/sandbox on` would refuse it — {denial}"),
    };
    ToolDecision {
        decision: "permission_preflight_only",
        reason: format!(
            "no isolation backend is configured, so nothing relocates this call and the normal \
             permission preflight decides it; in a session the /sandbox toggle is the only sandbox \
             gate and it starts off, and {when_on}. A workflow CLI run has no toggle and no \
             backend at all"
        ),
        provenance: None,
    }
}

/// Two things can be true of a name nothing declares, and both are worth
/// saying: this command builds a narrower registry than a session does, and the
/// mode's shell list can name something the registry does not have.
fn unknown_tool_report(tool: &str) -> String {
    let mut report = format!(
        "Decision: unknown_tool\nReason: no tool named {tool} is in the default registry this \
         command builds, so nothing declares a capability class for it. That registry is built \
         with no LEANN index and outside a session, so the semantic-search tools, and the tools a \
         session adds from MCP servers and plugins, are absent here even though a session would \
         have them\n"
    );
    if SandboxRouteMode::is_named_shell_tool(tool) {
        report.push_str(&format!(
            "Note: sandbox.mode names {tool} as shell execution and would route it to the backend, \
             so the mode and the registry disagree about whether this tool exists\n"
        ));
    }
    report
}

/// A tool's class lives on the tool and nowhere else, so the only honest route
/// from a CLI string to one is to build the registry the runtime builds and ask
/// it. It is not free — every built-in tool is constructed, ~175ms — but
/// explain runs once per invocation, while the cheap alternative is a name
/// table here that nothing keeps in step with the tools. That table is the
/// defect being removed.
///
/// `None` for the index rather than `session::init_leann_index`, which creates
/// `.archon/leann.db` and opens it. A command that only reports on a
/// configuration must not write to the project to do it; `unknown_tool_report`
/// says so instead.
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
