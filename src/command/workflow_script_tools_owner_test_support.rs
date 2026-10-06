use super::*;
use archon_tools::tool::{Tool, ToolResult};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Probe {
    runs: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    pending: bool,
}
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "OwnerProbe"
    }
    fn description(&self) -> &str {
        "Executor ownership probe"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self, _: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
    fn capability(&self) -> archon_permissions::ToolCapability {
        archon_permissions::ToolCapability::EXECUTION
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.pending {
            std::future::pending::<()>().await;
        }
        ToolResult::success("ran")
    }
}
pub(crate) fn host(
    pending: bool,
) -> (
    Arc<ScriptToolHost>,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Notify>,
) {
    let runs = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(Probe {
        runs: runs.clone(),
        entered: entered.clone(),
        pending,
    }));
    let checker = PermissionChecker::new(
        archon_permissions::mode::PermissionMode::default(),
        archon_permissions::rules::RuleSet {
            always_allow: vec![archon_permissions::rules::ToolRule {
                tool: "OwnerProbe".into(),
                pattern: "*".into(),
            }],
            ..Default::default()
        },
    );
    (
        Arc::new(ScriptToolHost {
            audited_writes: false,
            registry,
            checker,
            context: ToolContext::default(),
        }),
        runs,
        entered,
    )
}
