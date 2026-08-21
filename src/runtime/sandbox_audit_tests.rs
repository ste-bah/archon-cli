use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::*;

#[derive(Debug)]
struct FakeSandboxBackend {
    bash_result: Option<SandboxCommandResult>,
}

impl SandboxBackend for FakeSandboxBackend {
    fn check(
        &self,
        tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        if tool == "DenyMe" {
            Err("blocked".to_string())
        } else {
            Ok(())
        }
    }

    fn terminal(
        &self,
        _request: &archon_permissions::SandboxTerminalRequest,
    ) -> archon_permissions::SandboxTerminal {
        archon_permissions::SandboxTerminal::Open(archon_permissions::SandboxTerminalCommand {
            program: "fake".into(),
            args: vec!["shell".into()],
            shell: "bash".into(),
            location: "/workspace".into(),
        })
    }

    fn execute_bash<'a>(
        &'a self,
        _request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async move { self.bash_result.clone() })
    }
}

fn test_db() -> crate::command::test_db::TestDb<std::sync::Arc<cozo::DbInstance>> {
    crate::command::test_support::registered_learning_test_db("test-sandbox-audit")
}

#[tokio::test]
async fn wrapper_records_configured_session() {
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

    let wrapper = AuditedSandboxBackend::new_with_db(
        Arc::new(FakeSandboxBackend { bash_result: None }),
        config,
        archon_core::config::ArchonConfig::default(),
        "run-1".to_string(),
        "reviewer".to_string(),
        db.clone(),
    );
    let audit = wrapper.flush_audit().await;
    let sessions =
        archon_learning::sandbox_sessions::list_sandbox_sessions_by_status(&db, "configured")
            .unwrap();

    assert_eq!(audit.accepted, 1);
    assert_eq!(audit.persisted, 1);
    assert_eq!(audit.dropped, 0);
    assert_eq!(audit.failed, 0);
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
    let wrapper = AuditedSandboxBackend::new_with_db(
        Arc::new(FakeSandboxBackend {
            bash_result: Some(SandboxCommandResult {
                content: "ok".to_string(),
                is_error: false,
                exit_code: Some(0),
            }),
        }),
        config,
        archon_core::config::ArchonConfig::default(),
        "run-1".to_string(),
        "coder".to_string(),
        db.clone(),
    );

    wrapper
        .check(
            "Read",
            archon_permissions::ToolCapability::FILE_READ,
            &serde_json::json!({"path": "/secret"}),
        )
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
    let audit = wrapper.flush_audit().await;
    let events = archon_learning::sandbox_runtime_events::list_sandbox_runtime_events_by_backend(
        &db, "docker",
    )
    .unwrap();

    assert_eq!(audit.accepted, 3);
    assert_eq!(audit.persisted, 3);
    assert_eq!(audit.dropped, 0);
    assert_eq!(audit.failed, 0);
    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| event.decision == "allowed"));
    assert!(events.iter().any(|event| event.decision == "executed"));
    assert_eq!(events[0].agent_type.as_deref(), Some("coder"));
    assert!(events[0].redacted_context_json.get("command").is_none());
    assert!(events[0].redacted_context_json.get("env").is_none());
}

#[tokio::test]
async fn wrapper_feeds_denied_sandbox_events_into_agent_ledger() {
    let db = test_db();
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".to_string(),
        ..archon_core::sandbox::SandboxConfig::default()
    };
    let wrapper = AuditedSandboxBackend::new_with_db(
        Arc::new(FakeSandboxBackend { bash_result: None }),
        config,
        archon_core::config::ArchonConfig::default(),
        "run-2".to_string(),
        "reviewer".to_string(),
        db.clone(),
    );

    let error = wrapper
        .check(
            "DenyMe",
            archon_permissions::ToolCapability::HostLocal,
            &serde_json::json!({"command": "secret"}),
        )
        .unwrap_err();
    let audit = wrapper.flush_audit().await;
    let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        &db, "reviewer",
    )
    .unwrap();

    assert_eq!(audit.accepted, 2);
    assert_eq!(audit.persisted, 2);
    assert_eq!(audit.dropped, 0);
    assert_eq!(audit.failed, 0);
    assert_eq!(error, "blocked");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].gate_failed.as_deref(),
        Some("sandbox:openshell:denied")
    );
    assert!(
        rows[0]
            .evidence_ids
            .iter()
            .any(|evidence| evidence.starts_with("sandbox_event:sandbox-event-"))
    );
    assert!(
        rows[0]
            .evidence_ids
            .contains(&"sandbox_reason:sandbox_check_denied".to_string())
    );
}
