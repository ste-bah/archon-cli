//! Audit wrapper for session sandbox backends.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};
use cozo::DbInstance;

pub(crate) struct AuditedSandboxBackend {
    inner: Arc<dyn SandboxBackend>,
    config: archon_core::sandbox::SandboxConfig,
    run_id: String,
    agent_type: String,
    sandbox_session_id: String,
    db: Option<Arc<DbInstance>>,
}

impl std::fmt::Debug for AuditedSandboxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditedSandboxBackend")
            .field("inner", &self.inner)
            .field("backend", &self.config.backend)
            .field("run_id", &self.run_id)
            .field("agent_type", &self.agent_type)
            .field("sandbox_session_id", &self.sandbox_session_id)
            .field("db", &self.db.as_ref().map(|_| "<cozo>"))
            .finish()
    }
}

pub(crate) fn audit_sandbox_backend(
    inner: Arc<dyn SandboxBackend>,
    config: &archon_core::sandbox::SandboxConfig,
    run_id: impl Into<String>,
    agent_type: impl Into<String>,
) -> Arc<dyn SandboxBackend> {
    Arc::new(AuditedSandboxBackend::new(
        inner,
        config.clone(),
        run_id.into(),
        agent_type.into(),
        open_learning_db().ok().map(Arc::new),
    ))
}

impl AuditedSandboxBackend {
    fn new(
        inner: Arc<dyn SandboxBackend>,
        config: archon_core::sandbox::SandboxConfig,
        run_id: String,
        agent_type: String,
        db: Option<Arc<DbInstance>>,
    ) -> Self {
        let sandbox_session_id = format!("sandbox-session-{}", uuid::Uuid::new_v4());
        let backend = Self {
            inner,
            config,
            run_id,
            agent_type,
            sandbox_session_id,
            db,
        };
        backend.record_session("configured");
        backend
    }

    fn record_session(&self, status: &str) {
        let Some(db) = &self.db else {
            return;
        };
        let backend_kind = self.backend_kind();
        let mut session = archon_learning::sandbox_sessions::SandboxSessionRecord::new(
            self.sandbox_session_id.clone(),
            backend_kind.clone(),
            sandbox_profile_id(&self.config, &backend_kind),
            status,
            chrono::Utc::now().to_rfc3339(),
        )
        .with_run_context(Some(self.run_id.clone()), Some(self.agent_type.clone()))
        .with_workspace(workspace_mode(&self.config, &backend_kind), None)
        .with_transport(
            transport_kind(&backend_kind),
            transport_endpoint_redacted(&self.config, &backend_kind),
        );
        if backend_kind == "openshell" && self.config.openshell.provider_injection {
            session = session.with_provider_injection_enabled();
        }
        if let Err(error) = archon_learning::sandbox_sessions::insert_sandbox_session(db, &session)
        {
            tracing::warn!(%error, backend = %backend_kind, "sandbox session audit failed");
        }
    }

    fn record_event(&self, tool: &str, decision: &str, reason_code: &str) {
        let Some(db) = &self.db else {
            return;
        };
        let backend_kind = self.backend_kind();
        let event = archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord::new(
            format!("sandbox-event-{}", uuid::Uuid::new_v4()),
            backend_kind.clone(),
            decision,
            chrono::Utc::now().to_rfc3339(),
        )
        .with_run_context(Some(self.run_id.clone()), Some(self.agent_type.clone()))
        .with_tool(tool)
        .with_backend_instance(self.sandbox_session_id.clone())
        .with_policy(
            Some(reason_code.to_string()),
            Some(sandbox_profile_id(&self.config, &backend_kind)),
            workspace_mode(&self.config, &backend_kind),
            network_mode(&self.config, &backend_kind),
            Some(self.config.workspace_access.clone()),
        )
        .with_redacted_context(redacted_context(&self.config, &backend_kind));
        if let Err(error) =
            archon_learning::sandbox_runtime_events::insert_sandbox_runtime_event(db, &event)
        {
            tracing::warn!(%error, backend = %backend_kind, "sandbox runtime audit failed");
        }
    }

    fn backend_kind(&self) -> String {
        self.config
            .backend
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
    }
}

impl SandboxBackend for AuditedSandboxBackend {
    fn check(&self, tool: &str, input: &serde_json::Value) -> Result<(), String> {
        match self.inner.check(tool, input) {
            Ok(()) => {
                self.record_event(tool, "allowed", "sandbox_check_allowed");
                Ok(())
            }
            Err(error) => {
                self.record_event(tool, "denied", "sandbox_check_denied");
                Err(error)
            }
        }
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.inner.execute_bash(request).await;
            match &result {
                Some(result) if result.is_error => {
                    self.record_event("Bash", "failed", "sandbox_bash_error");
                }
                Some(_) => self.record_event("Bash", "executed", "sandbox_bash_ok"),
                None => self.record_event("Bash", "host_fallback", "sandbox_backend_delegated"),
            }
            result
        })
    }
}

fn open_learning_db() -> anyhow::Result<DbInstance> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    let path = parent.join("learning.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|error| anyhow::anyhow!("open learning db: {error}"))?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn sandbox_profile_id(config: &archon_core::sandbox::SandboxConfig, backend_kind: &str) -> String {
    format!(
        "config:{backend_kind}:{}:{}",
        config.mode, config.workspace_access
    )
}

fn workspace_mode(
    config: &archon_core::sandbox::SandboxConfig,
    backend_kind: &str,
) -> Option<String> {
    match backend_kind {
        "openshell" => Some(config.openshell.workspace_mode.clone()),
        "ssh" => Some(config.ssh.workspace_mode.clone()),
        _ => None,
    }
}

fn network_mode(
    config: &archon_core::sandbox::SandboxConfig,
    backend_kind: &str,
) -> Option<String> {
    match backend_kind {
        "docker" => Some(config.docker.network.clone()),
        _ => None,
    }
}

fn transport_kind(backend_kind: &str) -> Option<String> {
    match backend_kind {
        "docker" => Some("container".to_string()),
        "ssh" => Some("ssh".to_string()),
        "openshell" => Some("openshell".to_string()),
        _ => None,
    }
}

fn transport_endpoint_redacted(
    config: &archon_core::sandbox::SandboxConfig,
    backend_kind: &str,
) -> Option<String> {
    match backend_kind {
        "ssh" => config.ssh.host.as_deref().map(redact_endpoint),
        "openshell" => config.openshell.gateway.as_deref().map(redact_endpoint),
        _ => None,
    }
}

fn redact_endpoint(value: &str) -> String {
    if value.trim().is_empty() {
        return "[redacted]".to_string();
    }
    value
        .split_once('@')
        .map(|(_, host)| host)
        .unwrap_or(value)
        .split('/')
        .next()
        .map(|host| format!("{host}/[redacted]"))
        .unwrap_or_else(|| "[redacted]".to_string())
}

fn redacted_context(
    config: &archon_core::sandbox::SandboxConfig,
    backend_kind: &str,
) -> serde_json::Value {
    serde_json::json!({
        "source": "session_sandbox_backend",
        "backend": backend_kind,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeSandboxBackend {
        bash_result: Option<SandboxCommandResult>,
    }

    impl SandboxBackend for FakeSandboxBackend {
        fn check(&self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
            if tool == "DenyMe" {
                Err("blocked".to_string())
            } else {
                Ok(())
            }
        }

        fn execute_bash<'a>(
            &'a self,
            _request: SandboxCommandRequest,
        ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
            Box::pin(async move { self.bash_result.clone() })
        }
    }

    fn test_db() -> Arc<DbInstance> {
        let path = format!("/tmp/test-sandbox-audit-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        Arc::new(db)
    }

    #[test]
    fn wrapper_records_configured_session() {
        let db = test_db();
        let config = archon_core::sandbox::SandboxConfig {
            backend: "openshell".to_string(),
            openshell: archon_core::sandbox::OpenShellConfig {
                workspace_mode: "mirror".to_string(),
                gateway: Some("user@gateway.example/private".to_string()),
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let wrapper = AuditedSandboxBackend::new(
            Arc::new(FakeSandboxBackend { bash_result: None }),
            config,
            "run-1".to_string(),
            "reviewer".to_string(),
            Some(db.clone()),
        );
        let sessions =
            archon_learning::sandbox_sessions::list_sandbox_sessions_by_status(&db, "configured")
                .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].backend_kind, "openshell");
        assert_eq!(sessions[0].agent_type.as_deref(), Some("reviewer"));
        assert_eq!(sessions[0].workspace_mode.as_deref(), Some("mirror"));
        assert_eq!(
            sessions[0].transport_endpoint_redacted.as_deref(),
            Some("gateway.example/[redacted]")
        );
        assert!(wrapper.sandbox_session_id.starts_with("sandbox-session-"));
    }

    #[tokio::test]
    async fn wrapper_records_redacted_check_and_bash_events() {
        let db = test_db();
        let config = archon_core::sandbox::SandboxConfig {
            backend: "docker".to_string(),
            ..archon_core::sandbox::SandboxConfig::default()
        };
        let wrapper = AuditedSandboxBackend::new(
            Arc::new(FakeSandboxBackend {
                bash_result: Some(SandboxCommandResult {
                    content: "ok".to_string(),
                    is_error: false,
                }),
            }),
            config,
            "run-1".to_string(),
            "coder".to_string(),
            Some(db.clone()),
        );

        wrapper
            .check("Read", &serde_json::json!({"path": "/secret"}))
            .unwrap();
        wrapper
            .execute_bash(SandboxCommandRequest {
                command: "echo secret".to_string(),
                working_dir: ".".into(),
                timeout_ms: 1_000,
                max_output_bytes: 1024,
                env: vec![("TOKEN".to_string(), "secret".to_string())],
            })
            .await;
        let events =
            archon_learning::sandbox_runtime_events::list_sandbox_runtime_events_by_backend(
                &db, "docker",
            )
            .unwrap();

        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| event.decision == "allowed"));
        assert!(events.iter().any(|event| event.decision == "executed"));
        assert_eq!(events[0].agent_type.as_deref(), Some("coder"));
        assert!(events[0].redacted_context_json.get("command").is_none());
        assert!(events[0].redacted_context_json.get("env").is_none());
    }
}
