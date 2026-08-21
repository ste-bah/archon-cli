//! Audit wrapper for session sandbox backends.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::runtime::learning_store;
#[cfg(test)]
use crate::runtime::sandbox_audit_writer::SandboxAuditReadback;
use crate::runtime::sandbox_audit_writer::{
    SandboxAuditDrain, SandboxAuditWrite, SandboxAuditWriter,
};
use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};
#[cfg(test)]
use cozo::DbInstance;

pub(crate) struct AuditedSandboxBackend {
    inner: Arc<dyn SandboxBackend>,
    config: archon_core::sandbox::SandboxConfig,
    archon_config: archon_core::config::ArchonConfig,
    run_id: String,
    agent_type: String,
    sandbox_session_id: String,
    writer: Option<SandboxAuditWriter>,
}

impl std::fmt::Debug for AuditedSandboxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditedSandboxBackend")
            .field("inner", &self.inner)
            .field("backend", &self.config.backend)
            .field("run_id", &self.run_id)
            .field("agent_type", &self.agent_type)
            .field("sandbox_session_id", &self.sandbox_session_id)
            .field("writer", &self.writer.as_ref().map(|_| "<sandbox-audit>"))
            .finish()
    }
}

pub(crate) async fn audit_sandbox_backend(
    inner: Arc<dyn SandboxBackend>,
    config: &archon_core::config::ArchonConfig,
    run_id: impl Into<String>,
    agent_type: impl Into<String>,
) -> anyhow::Result<(Arc<dyn SandboxBackend>, SandboxAuditDrain)> {
    let run_id = run_id.into();
    let agent_type = agent_type.into();
    let db = learning_store::acquire_default_async()
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "sandbox audit persistence unavailable for run {run_id} agent {agent_type}: {error}"
            )
        })?;
    let (writer, drain) = SandboxAuditWriter::new(db);
    Ok((
        Arc::new(AuditedSandboxBackend::new(
            inner,
            config.sandbox.clone(),
            config.clone(),
            run_id,
            agent_type,
            Some(writer),
        )),
        drain,
    ))
}

impl AuditedSandboxBackend {
    #[cfg(test)]
    fn new_with_db(
        inner: Arc<dyn SandboxBackend>,
        config: archon_core::sandbox::SandboxConfig,
        archon_config: archon_core::config::ArchonConfig,
        run_id: String,
        agent_type: String,
        db: Arc<DbInstance>,
    ) -> Self {
        let (writer, _drain) = SandboxAuditWriter::new(db);
        Self::new(
            inner,
            config,
            archon_config,
            run_id,
            agent_type,
            Some(writer),
        )
    }

    fn new(
        inner: Arc<dyn SandboxBackend>,
        config: archon_core::sandbox::SandboxConfig,
        archon_config: archon_core::config::ArchonConfig,
        run_id: String,
        agent_type: String,
        writer: Option<SandboxAuditWriter>,
    ) -> Self {
        let sandbox_session_id = format!("sandbox-session-{}", uuid::Uuid::new_v4());
        let backend = Self {
            inner,
            config,
            archon_config,
            run_id,
            agent_type,
            sandbox_session_id,
            writer,
        };
        backend.record_session("configured");
        backend
    }

    fn record_session(&self, status: &str) {
        let Some(writer) = &self.writer else {
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
        writer.enqueue(SandboxAuditWrite::Session(Box::new(session)));
    }

    fn record_event(&self, tool: &str, decision: &str, reason_code: &str) {
        let Some(writer) = &self.writer else {
            return;
        };
        let backend_kind = self.backend_kind();
        let event_id = format!("sandbox-event-{}", uuid::Uuid::new_v4());
        let event = archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord::new(
            event_id.clone(),
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
        let ledger = self.agent_ledger_signal(&event_id, decision, reason_code, &backend_kind);
        writer.enqueue(SandboxAuditWrite::RuntimeEvent {
            event: Box::new(event),
            ledger: ledger.map(Box::new),
        });
    }

    fn agent_ledger_signal(
        &self,
        sandbox_event_id: &str,
        decision: &str,
        reason_code: &str,
        backend_kind: &str,
    ) -> Option<archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord> {
        if !matches!(decision, "denied" | "failed") {
            return None;
        }
        let mut record =
            archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
                format!("ledger-{}", uuid::Uuid::new_v4()),
                self.agent_type.clone(),
                "failed",
                chrono::Utc::now().to_rfc3339(),
            )
            .with_run_id(self.run_id.clone())
            .add_evidence(format!("sandbox_event:{sandbox_event_id}"))
            .add_evidence(format!("sandbox_reason:{reason_code}"));
        record.gate_failed = Some(format!("sandbox:{backend_kind}:{decision}"));
        record.completion_rate = Some(0.0);
        Some(record)
    }

    #[cfg(test)]
    async fn flush_audit(&self) -> SandboxAuditReadback {
        match &self.writer {
            Some(writer) => writer.flush_for_test().await,
            None => SandboxAuditReadback::default(),
        }
    }

    fn backend_kind(&self) -> String {
        self.config
            .backend
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
    }

    fn record_world_guardrail_tool_result(
        &self,
        request: &SandboxCommandRequest,
        result: Option<&SandboxCommandResult>,
    ) {
        let Some(result) = result else {
            return;
        };
        let Some(active) = crate::command::world_model::active_guardrail_for_session(&self.run_id)
        else {
            return;
        };
        let is_error = result.is_error;
        let output_summary = result.content.chars().take(500).collect::<String>();
        tracing::debug!(
            parent_action_id = %active.action.action_id,
            command = %request.command,
            is_error,
            "world_model.guardrail_tool_result"
        );
        let _ = crate::command::world_model::record_guardrail_tool_result_for_session(
            &self.archon_config,
            &self.run_id,
            &request.command,
            is_error,
            &output_summary,
        );
    }
}

impl SandboxBackend for AuditedSandboxBackend {
    fn check(
        &self,
        tool: &str,
        capability: archon_permissions::ToolCapability,
        input: &serde_json::Value,
    ) -> Result<(), String> {
        match self.inner.check(tool, capability, input) {
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
            let result = self.inner.execute_bash(request.clone()).await;
            match &result {
                Some(result) if result.is_error => {
                    self.record_event("Bash", "failed", "sandbox_bash_error");
                }
                Some(_) => self.record_event("Bash", "executed", "sandbox_bash_ok"),
                None => self.record_event("Bash", "host_fallback", "sandbox_backend_delegated"),
            }
            self.record_world_guardrail_tool_result(&request, result.as_ref());
            result
        })
    }
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
#[path = "sandbox_audit_tests.rs"]
mod tests;
