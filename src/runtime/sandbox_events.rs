//! Sandbox audit-event bridge for the governed learning Cozo store.

use crate::runtime::learning_store;
use anyhow::Result;

pub(crate) fn record_sandbox_cli_event(
    config: &archon_core::sandbox::SandboxConfig,
    backend_override: Option<&str>,
    decision: &str,
    reason_code: &str,
) -> Result<()> {
    let event = build_sandbox_cli_event(config, backend_override, decision, reason_code)?;
    let db = learning_store::acquire_default()?;
    archon_learning::sandbox_runtime_events::insert_sandbox_runtime_event(&db, &event)
}

fn build_sandbox_cli_event(
    config: &archon_core::sandbox::SandboxConfig,
    backend_override: Option<&str>,
    decision: &str,
    reason_code: &str,
) -> Result<archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend_override) = backend_override {
        policy.backend = backend_override.parse().map_err(anyhow::Error::msg)?;
    }
    let backend = policy.backend.to_string();
    let context = redacted_context(config, &backend);
    Ok(
        archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord::new(
            format!("sandbox-event-{}", uuid::Uuid::new_v4()),
            backend,
            decision,
            chrono::Utc::now().to_rfc3339(),
        )
        .with_policy(
            Some(reason_code.to_string()),
            None,
            workspace_mode(config, policy.backend),
            network_mode(config, policy.backend),
            Some(policy.workspace_access),
        )
        .with_redacted_context(context),
    )
}

fn redacted_context(
    config: &archon_core::sandbox::SandboxConfig,
    backend: &str,
) -> serde_json::Value {
    serde_json::json!({
        "source": "sandbox_cli",
        "backend": backend,
        "mode": config.mode,
        "scope": config.scope,
        "workspace_access": config.workspace_access,
        "openshell_provider_injection": config.openshell.provider_injection,
        "openshell_host_shell_fallback": config.openshell.host_shell_fallback,
        "docker_privileged": config.docker.privileged,
        "docker_mount_home": config.docker.mount_home,
        "docker_mount_socket": config.docker.mount_docker_socket
    })
}

fn workspace_mode(
    config: &archon_core::sandbox::SandboxConfig,
    backend: archon_core::sandbox::SandboxBackendKind,
) -> Option<String> {
    match backend {
        archon_core::sandbox::SandboxBackendKind::OpenShell => {
            Some(config.openshell.workspace_mode.clone())
        }
        archon_core::sandbox::SandboxBackendKind::Ssh => Some(config.ssh.workspace_mode.clone()),
        _ => None,
    }
}

fn network_mode(
    config: &archon_core::sandbox::SandboxConfig,
    backend: archon_core::sandbox::SandboxBackendKind,
) -> Option<String> {
    match backend {
        archon_core::sandbox::SandboxBackendKind::Docker => Some(config.docker.network.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_cli_event_redacts_backend_policy_without_credentials() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "openshell".to_string(),
            workspace_access: "rw".to_string(),
            openshell: archon_core::sandbox::OpenShellConfig {
                workspace_mode: "mirror".to_string(),
                provider_injection: false,
                host_shell_fallback: false,
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let event = build_sandbox_cli_event(&config, None, "explain", "cli_explain").unwrap();

        assert_eq!(event.backend_kind, "openshell");
        assert_eq!(event.workspace_mode.as_deref(), Some("mirror"));
        assert_eq!(event.workspace_mount_mode.as_deref(), Some("rw"));
        assert_eq!(event.reason_code.as_deref(), Some("cli_explain"));
        assert_eq!(event.redacted_context_json["source"], "sandbox_cli");
        assert_eq!(
            event.redacted_context_json["openshell_provider_injection"],
            false
        );
        assert!(event.redacted_context_json.get("gateway").is_none());
        assert!(event.redacted_context_json.get("api_key").is_none());
    }

    #[test]
    fn sandbox_cli_event_backend_override_sets_backend_policy() {
        let config = archon_core::sandbox::SandboxConfig::default();

        let event =
            build_sandbox_cli_event(&config, Some("docker"), "test_config_valid", "cli_test")
                .unwrap();

        assert_eq!(event.backend_kind, "docker");
        assert_eq!(event.network_mode.as_deref(), Some("disabled"));
    }
}
