use anyhow::Result;

use crate::cli_args::SandboxAction;

pub(crate) fn handle_sandbox_command(
    action: Option<SandboxAction>,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let action = action.unwrap_or(SandboxAction::Status { verbose: false });
    let output = match action {
        SandboxAction::Status { verbose } => {
            let output = render_status(&config.sandbox, verbose)?;
            persist_sandbox_command_event(&config.sandbox, None, "status", "cli_status");
            output
        }
        SandboxAction::Explain { backend } => {
            let output = render_explain(&config.sandbox, backend.clone())?;
            persist_sandbox_command_event(
                &config.sandbox,
                backend.as_deref(),
                "explain",
                "cli_explain",
            );
            output
        }
        SandboxAction::Doctor { backend } => crate::command::sandbox_doctor::render_sandbox_doctor(
            &doctor_args(backend),
            crate::command::sandbox_doctor::SandboxDoctorOverrides::default(),
        ),
        SandboxAction::Test { backend } => {
            let output = render_test(&config.sandbox, backend.clone())?;
            persist_sandbox_command_event(
                &config.sandbox,
                backend.as_deref(),
                "test_config_valid",
                "cli_test",
            );
            output
        }
    };
    print!("{output}");
    Ok(())
}

fn render_status(config: &archon_core::sandbox::SandboxConfig, verbose: bool) -> Result<String> {
    let policy = config.policy().map_err(anyhow::Error::msg)?;
    let mut output = format!(
        "Sandbox status\nBackend: {}\nMode: {}\nScope: {}\nWorkspace access: {}\nIsolation: {}\n",
        policy.backend,
        policy.mode,
        policy.scope,
        policy.workspace_access,
        policy.describes_isolation()
    );
    if verbose {
        output.push_str(
            "Compatibility: /sandbox toggles the logical TUI gate only; normal permission rules still apply\n",
        );
        output.push_str(
            "Execution: docker can route Bash when selected; ssh and openshell fail closed until execution is implemented\n",
        );
    }
    Ok(output)
}

fn render_explain(
    config: &archon_core::sandbox::SandboxConfig,
    backend: Option<String>,
) -> Result<String> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend) = backend {
        policy.backend = backend.parse().map_err(anyhow::Error::msg)?;
    }
    Ok(format!(
        "Sandbox explain\nBackend: {}\nIsolation: {}\nDecision flow: UnifiedToolPreflight -> PermissionChecker -> SandboxPolicyResolver -> SandboxBackend -> ToolDispatch\nPermissions: sandbox policy cannot bypass always_deny rules, permission modes, or dangerous-bypass guards\nExecution: docker can route Bash when selected; ssh and openshell fail closed instead of falling back to host shell\n",
        policy.backend,
        policy.describes_isolation()
    ))
}

fn render_test(
    config: &archon_core::sandbox::SandboxConfig,
    backend: Option<String>,
) -> Result<String> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend) = backend {
        policy.backend = backend.parse().map_err(anyhow::Error::msg)?;
    }
    policy.validate().map_err(anyhow::Error::msg)?;
    Ok(format!(
        "Sandbox test\nBackend: {}\nConfig: valid\nExecution: detect-only; no untrusted command was run\n",
        policy.backend
    ))
}

fn doctor_args(backend: Option<String>) -> Vec<String> {
    match backend {
        Some(backend) => vec!["--backend".into(), backend],
        None => Vec::new(),
    }
}

fn persist_sandbox_command_event(
    config: &archon_core::sandbox::SandboxConfig,
    backend_override: Option<&str>,
    decision: &str,
    reason_code: &str,
) {
    if let Err(error) = crate::runtime::sandbox_events::record_sandbox_cli_event(
        config,
        backend_override,
        decision,
        reason_code,
    ) {
        tracing::warn!(%error, decision, "sandbox audit event persistence failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_status_shows_policy_fields() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "docker".into(),
            mode: "all".into(),
            workspace_access: "rw".into(),
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_status(&config, true).unwrap();

        assert!(body.contains("Backend: docker"));
        assert!(body.contains("Mode: all"));
        assert!(body.contains("Workspace access: rw"));
        assert!(body.contains("normal permission rules still apply"));
    }

    #[test]
    fn sandbox_explain_rejects_unknown_backend_override() {
        let config = archon_core::sandbox::SandboxConfig::default();

        let error = render_explain(&config, Some("host".into())).unwrap_err();

        assert!(error.to_string().contains("sandbox.backend"));
    }

    #[test]
    fn sandbox_test_is_detect_only() {
        let config = archon_core::sandbox::SandboxConfig::default();

        let body = render_test(&config, Some("openshell".into())).unwrap();

        assert!(body.contains("Backend: openshell"));
        assert!(body.contains("no untrusted command was run"));
    }
}
