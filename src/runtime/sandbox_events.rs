//! Sandbox audit-event bridge for the governed learning Cozo store.

use anyhow::Result;
use cozo::DbInstance;

pub(crate) fn record_sandbox_cli_event(
    config: &archon_core::sandbox::SandboxConfig,
    backend_override: Option<&str>,
    decision: &str,
    reason_code: &str,
) -> Result<()> {
    let event = build_sandbox_cli_event(config, backend_override, decision, reason_code)?;
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
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

fn learning_db_path() -> Result<std::path::PathBuf> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(parent.join("learning.db"))
}

fn open_learning_db(path: &std::path::Path) -> Result<DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    DbInstance::new("sqlite", &path_str, "").map_err(|e| anyhow::anyhow!("open learning db: {e}"))
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
