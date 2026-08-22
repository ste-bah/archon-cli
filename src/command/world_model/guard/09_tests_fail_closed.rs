use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

struct AdmissionFailureTestTool {
    executions: Arc<AtomicUsize>,
    permission_level: PermissionLevel,
}

#[async_trait::async_trait]
impl Tool for AdmissionFailureTestTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::EXECUTION
    }

    fn description(&self) -> &str {
        "admission failure test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("executed")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        self.permission_level
    }
}

fn fail_closed_tool_run_config() -> archon_core::config::ArchonConfig {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.guardrails.tool_run_mode = "guarded".into();
    config
}

fn fail_closed_tool_run_request() -> ToolRunAdmissionRequest {
    ToolRunAdmissionRequest {
        session_id: "fail-closed-session".into(),
        parent_action_id: "parent-action".into(),
        tool_use_id: "tool-use-1".into(),
        attempt: 0,
        tool_name: "Bash".into(),
        input: serde_json::json!({"command": "dangerous command"}),
        permission_level: PermissionLevel::Dangerous,
    }
}

#[test]
fn initial_guardrail_persistence_failure_rejects_admission() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let record = runtime_guardrail_record("initial-fail-session", "initial-fail-action");

    let result = admit_guardrail_record_at_root(temp.path(), record);

    assert!(result.is_err());
    assert!(active_guardrail_for_session("initial-fail-session").is_none());
}

#[test]
fn initial_guardrail_root_failure_rejects_admission() {
    let config = guarded_test_config();

    let result = begin_guarded_action_with_root(
        &config,
        archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
        "root-fail-session",
        "root-fail-action",
        "fix bug",
        Err(anyhow::anyhow!("home directory unavailable")),
    );

    assert!(result.is_err());
    assert!(active_guardrail_for_session("root-fail-session").is_none());
}

#[test]
fn advisory_initial_guardrail_root_failure_remains_non_blocking() {
    let mut config = guarded_test_config();
    config.learning.world_model.guardrails.interactive_mode = "advisory".into();

    let result = begin_guarded_action_with_root(
        &config,
        archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
        "advisory-root-fail-session",
        "advisory-root-fail-action",
        "fix bug",
        Err(anyhow::anyhow!("home directory unavailable")),
    );

    assert!(matches!(result, Ok(None)));
}

#[test]
fn tool_run_candidate_storage_failure_blocks_attempt() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("world-model.db")).unwrap();

    let verdict = admit_tool_run_attempt_at_root(
        &fail_closed_tool_run_config(),
        temp.path(),
        fail_closed_tool_run_request(),
    );

    assert!(matches!(verdict, ToolRunAdmission::Blocked { .. }));
}

#[test]
fn tool_run_root_failure_blocks_attempt() {
    let verdict = admit_tool_run_attempt_with_root(
        &fail_closed_tool_run_config(),
        Err(anyhow::anyhow!("home directory unavailable")),
        fail_closed_tool_run_request(),
    );

    assert!(matches!(verdict, ToolRunAdmission::Blocked { .. }));
}

#[test]
fn tool_run_revision_storage_failure_blocks_attempt() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let request = fail_closed_tool_run_request();

    let verdict = admit_tool_run_at_root(
        &fail_closed_tool_run_config(),
        temp.path(),
        &request,
        critical_test_advisory(&request),
    );

    assert!(matches!(verdict, ToolRunAdmission::Blocked { .. }));
}

#[tokio::test]
async fn admission_storage_failure_prevents_dangerous_tool_execution() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let config = fail_closed_tool_run_config();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = archon_core::dispatch::ToolRegistry::new();
    registry.register(Box::new(AdmissionFailureTestTool {
        executions: Arc::clone(&executions),
        permission_level: PermissionLevel::Dangerous,
    }));
    let admission_root = temp.path().to_path_buf();
    let admission_config = config.clone();
    let ctx = ToolContext {
        session_id: "fail-closed-session".into(),
        tool_run_parent_action_id: Some("parent-action".into()),
        tool_run_tool_use_id: Some("tool-use-1".into()),
        tool_run_admission: Some(Arc::new(move |request| {
            admit_tool_run_at_root(
                &admission_config,
                &admission_root,
                &request,
                critical_test_advisory(&request),
            )
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch(
            "Bash",
            serde_json::json!({"command": "dangerous command"}),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("admission storage"));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admission_storage_failure_prevents_risky_tool_execution() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = archon_core::dispatch::ToolRegistry::new();
    registry.register(Box::new(AdmissionFailureTestTool {
        executions: Arc::clone(&executions),
        permission_level: PermissionLevel::Risky,
    }));
    let admission_root = temp.path().to_path_buf();
    let admission_config = fail_closed_tool_run_config();
    let ctx = ToolContext {
        tool_run_admission: Some(Arc::new(move |request| {
            admit_tool_run_at_root(
                &admission_config,
                &admission_root,
                &request,
                critical_test_advisory(&request),
            )
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch(
            "Bash",
            serde_json::json!({"command": "risky command"}),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[test]
fn advisory_tool_run_guardrail_storage_failure_remains_non_blocking() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("world-model.db")).unwrap();
    let mut config = fail_closed_tool_run_config();
    config.learning.world_model.guardrails.tool_run_mode = "advisory".into();

    assert_eq!(
        admit_tool_run_attempt_at_root(&config, temp.path(), fail_closed_tool_run_request()),
        ToolRunAdmission::Allowed
    );
}

#[test]
fn learn_only_tool_run_guardrail_storage_failure_remains_non_blocking() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("world-model.db")).unwrap();
    let mut config = fail_closed_tool_run_config();
    config.learning.world_model.guardrails.tool_run_mode = "learn_only".into();

    assert_eq!(
        admit_tool_run_attempt_at_root(&config, temp.path(), fail_closed_tool_run_request()),
        ToolRunAdmission::Allowed
    );
}

#[test]
fn disabled_tool_run_guardrail_still_bypasses_storage() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("world-model.db")).unwrap();
    let mut config = fail_closed_tool_run_config();
    config.learning.world_model.guardrails.enabled = false;

    assert_eq!(
        admit_tool_run_attempt_at_root(&config, temp.path(), fail_closed_tool_run_request()),
        ToolRunAdmission::Allowed
    );
}

#[test]
fn off_tool_run_guardrail_still_bypasses_storage() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("world-model.db")).unwrap();
    let mut config = fail_closed_tool_run_config();
    config.learning.world_model.guardrails.tool_run_mode = "off".into();

    assert_eq!(
        admit_tool_run_attempt_at_root(&config, temp.path(), fail_closed_tool_run_request()),
        ToolRunAdmission::Allowed
    );
}
