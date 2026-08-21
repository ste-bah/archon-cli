use super::*;
use archon_tools::tool::{
    PermissionLevel, ToolRunAdmission, ToolRunAdmissionRequest, ToolRunAttemptOutcome,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct AdmissionTestTool {
    permission: PermissionLevel,
    executions: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct DenyAllSandbox;

impl archon_permissions::SandboxBackend for DenyAllSandbox {
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Err("sandbox denied".into())
    }
}

#[async_trait::async_trait]
impl Tool for AdmissionTestTool {
    fn name(&self) -> &str {
        "AdmissionTest"
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "tool-run admission test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("executed")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        self.permission
    }
}

#[tokio::test]
async fn dispatch_blocks_risky_tool_before_execution_and_records_attempt() {
    let executions = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(AdmissionTestTool {
        permission: PermissionLevel::Risky,
        executions: Arc::clone(&executions),
    }));
    let request_log = Arc::clone(&requests);
    let outcome_log = Arc::clone(&outcomes);
    let ctx = ToolContext {
        session_id: "session-1".into(),
        tool_run_parent_action_id: Some("parent-1".into()),
        tool_run_tool_use_id: Some("tool-use-1".into()),
        tool_run_attempt: 2,
        tool_run_admission: Some(Arc::new(move |request: ToolRunAdmissionRequest| {
            request_log.lock().unwrap().push(request);
            ToolRunAdmission::Blocked {
                reason: "critical ToolRun risk".into(),
            }
        })),
        tool_run_outcome: Some(Arc::new(move |outcome: ToolRunAttemptOutcome| {
            outcome_log.lock().unwrap().push(outcome);
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch("AdmissionTest", serde_json::json!({"value": 1}), &ctx)
        .await;

    assert!(result.is_error);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].parent_action_id, "parent-1");
    assert_eq!(requests[0].tool_use_id, "tool-use-1");
    assert_eq!(requests[0].attempt, 2);
    assert_eq!(requests[0].permission_level, PermissionLevel::Risky);
    let outcomes = outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].blocked);
    assert_eq!(outcomes[0].attempt, 2);
    assert_eq!(outcomes[0].input, serde_json::json!({"value": 1}));
}

#[tokio::test]
async fn sandbox_denied_risky_tool_is_admitted_and_records_one_outcome() {
    let executions = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(AdmissionTestTool {
        permission: PermissionLevel::Risky,
        executions: Arc::clone(&executions),
    }));
    let request_log = Arc::clone(&requests);
    let outcome_log = Arc::clone(&outcomes);
    let ctx = ToolContext {
        sandbox: Some(Arc::new(DenyAllSandbox)),
        session_id: "session-1".into(),
        tool_run_parent_action_id: Some("parent-1".into()),
        tool_run_tool_use_id: Some("tool-use-1".into()),
        tool_run_admission: Some(Arc::new(move |request| {
            request_log.lock().unwrap().push(request);
            ToolRunAdmission::Allowed
        })),
        tool_run_outcome: Some(Arc::new(move |outcome| {
            outcome_log.lock().unwrap().push(outcome);
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch("AdmissionTest", serde_json::json!({}), &ctx)
        .await;

    assert!(result.is_error);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(requests.lock().unwrap().len(), 1);
    let outcomes = outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].blocked);
    assert!(outcomes[0].is_error);
}

#[tokio::test]
async fn dispatch_safe_tool_bypasses_admission_but_still_reports_its_outcome() {
    // The admission *policy* still exempts `Safe` tools. The outcome callback
    // no longer inherits that exemption — ambient topology tracing needs every
    // attempt, and `Safe` is the great majority of them. `admission_evaluated`
    // is how a consumer that cares about the distinction recovers it.
    let executions = Arc::new(AtomicUsize::new(0));
    let admissions = Arc::new(AtomicUsize::new(0));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(AdmissionTestTool {
        permission: PermissionLevel::Safe,
        executions: Arc::clone(&executions),
    }));
    let admission_count = Arc::clone(&admissions);
    let outcome_log = Arc::clone(&outcomes);
    let ctx = ToolContext {
        tool_run_admission: Some(Arc::new(move |_| {
            admission_count.fetch_add(1, Ordering::SeqCst);
            ToolRunAdmission::Blocked {
                reason: "must not run".into(),
            }
        })),
        tool_run_outcome: Some(Arc::new(move |outcome: ToolRunAttemptOutcome| {
            outcome_log.lock().unwrap().push(outcome);
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch("AdmissionTest", serde_json::json!({}), &ctx)
        .await;

    assert!(!result.is_error);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(
        admissions.load(Ordering::SeqCst),
        0,
        "a Safe tool must never reach the admission callback"
    );
    let outcomes = outcomes.lock().unwrap();
    assert_eq!(
        outcomes.len(),
        1,
        "a Safe tool must still report an outcome"
    );
    assert_eq!(outcomes[0].permission_level, PermissionLevel::Safe);
    assert!(!outcomes[0].blocked);
    assert!(!outcomes[0].is_error);
    assert!(
        !outcomes[0].admission_evaluated,
        "no admission ran, so the outcome must say so"
    );
}

#[tokio::test]
async fn outcome_is_reported_when_no_admission_callback_is_installed() {
    // The other half of the old filter: `permission_level != Safe` *and* a
    // callback installed. A risky tool with no admission callback used to
    // report nothing at all.
    let executions = Arc::new(AtomicUsize::new(0));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(AdmissionTestTool {
        permission: PermissionLevel::Risky,
        executions: Arc::clone(&executions),
    }));
    let outcome_log = Arc::clone(&outcomes);
    let ctx = ToolContext {
        tool_run_admission: None,
        tool_run_outcome: Some(Arc::new(move |outcome: ToolRunAttemptOutcome| {
            outcome_log.lock().unwrap().push(outcome);
        })),
        ..ToolContext::default()
    };

    let result = registry
        .dispatch("AdmissionTest", serde_json::json!({}), &ctx)
        .await;

    assert!(!result.is_error);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let outcomes = outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].permission_level, PermissionLevel::Risky);
    assert!(!outcomes[0].admission_evaluated);
}

#[tokio::test]
async fn an_admitted_attempt_reports_admission_evaluated() {
    let executions = Arc::new(AtomicUsize::new(0));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(AdmissionTestTool {
        permission: PermissionLevel::Risky,
        executions: Arc::clone(&executions),
    }));
    let outcome_log = Arc::clone(&outcomes);
    let ctx = ToolContext {
        tool_run_admission: Some(Arc::new(|_| ToolRunAdmission::Allowed)),
        tool_run_outcome: Some(Arc::new(move |outcome: ToolRunAttemptOutcome| {
            outcome_log.lock().unwrap().push(outcome);
        })),
        ..ToolContext::default()
    };

    let _ = registry
        .dispatch("AdmissionTest", serde_json::json!({}), &ctx)
        .await;

    let outcomes = outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].admission_evaluated);
}
