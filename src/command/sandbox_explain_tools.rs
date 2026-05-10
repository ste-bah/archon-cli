pub(super) fn append_tool_explain(
    output: &mut String,
    policy: &archon_core::sandbox::SandboxPolicy,
    tool: Option<&str>,
    command: Option<&str>,
) {
    let Some(tool) = tool.map(str::trim).filter(|tool| !tool.is_empty()) else {
        return;
    };
    let decision = tool_decision(policy, tool);
    output.push_str(&format!(
        "Tool explain\nTool: {tool}\nDecision: {}\nReason: {}\n",
        decision.0, decision.1
    ));
    if let Some(command) = command.map(str::trim).filter(|value| !value.is_empty()) {
        output.push_str(&format!("Command preview: {}\n", command_preview(command)));
    }
}

fn tool_decision(
    policy: &archon_core::sandbox::SandboxPolicy,
    tool: &str,
) -> (&'static str, &'static str) {
    match (policy.backend, tool) {
        (archon_core::sandbox::SandboxBackendKind::Docker, "Bash" | "Shell") => (
            "route_to_sandbox",
            "Docker can execute approved shell commands with configured mounts, network, and resource limits",
        ),
        (archon_core::sandbox::SandboxBackendKind::Ssh, "Bash" | "Shell") => (
            "route_to_sandbox",
            "SSH can execute approved shell commands on the configured remote sandbox target",
        ),
        (archon_core::sandbox::SandboxBackendKind::OpenShell, "Bash" | "Shell") => (
            "route_to_sandbox",
            "OpenShell can execute approved shell commands through its sandbox lifecycle",
        ),
        (archon_core::sandbox::SandboxBackendKind::Logical, _) => (
            "policy_check_only",
            "logical sandbox applies tool policy but does not provide process isolation",
        ),
        (archon_core::sandbox::SandboxBackendKind::Disabled, _) => (
            "permission_preflight_only",
            "sandbox backend is disabled; normal permission preflight still applies",
        ),
        (_, "PowerShell") if policy.backend.is_real_isolation() && policy.mode != "all" => (
            "blocked_shell_not_supported",
            "sandbox mode routes shell execution through Bash-compatible backends; PowerShell cannot be sandbox-routed yet",
        ),
        (_, _) if policy.backend.is_real_isolation() && policy.mode != "all" => (
            "permission_preflight_host_tool",
            "sandbox.mode routes shell execution through the backend while non-shell tools continue through normal permission preflight",
        ),
        (_, "Write" | "Edit" | "NotebookEdit") => (
            "host_mutation_blocked_by_backend",
            "sandbox.mode=all requires backend-compatible tools and does not allow host-side file mutation tools",
        ),
        (_, "WebFetch" | "WebSearch") => (
            "host_network_blocked_by_backend",
            "sandbox.mode=all requires backend-compatible tools and does not allow host-side network tools",
        ),
        (_, "TaskCreate" | "TaskUpdate" | "Agent") => (
            "agent_spawn_blocked_by_backend",
            "sandbox.mode=all requires backend-compatible tools and does not allow host-side agent spawning",
        ),
        _ => (
            "permission_preflight_required",
            "tool still needs the normal permission and sandbox preflight checks",
        ),
    }
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
