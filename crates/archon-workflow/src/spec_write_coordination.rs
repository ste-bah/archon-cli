//! PRD-012 spec-level guard for coordinated implementation fanout.

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout;
use crate::spec::{StageKind, WorkflowSpec};
use crate::write_coordinator::WriteCoordinatorConfig;

impl WorkflowSpec {
    /// Implementation fanout items must declare per-item targets so the
    /// conflict graph can schedule them into waves.
    ///
    /// Only inline `input.items` are checked — foreach items materialize at
    /// runtime from a producer stage, so undeclared writes there are caught by
    /// the patch-manifest validation instead of failing every foreach spec.
    pub fn validate_write_coordination(
        &self,
        cfg: &WriteCoordinatorConfig,
    ) -> WorkflowResult<()> {
        if !cfg.enabled || !cfg.fail_on_undeclared_write {
            return Ok(());
        }
        for stage in &self.stages {
            if stage.effective_item_kind() != StageKind::Implementation {
                continue;
            }
            if stage
                .input
                .get("items")
                .and_then(serde_json::Value::as_array)
                .is_none()
            {
                continue;
            }
            for item in fanout::extract_items(stage) {
                if !item_declares_targets(&item.payload) {
                    return Err(WorkflowError::ImplementationFanoutMissingPerItemTargets {
                        stage: stage.id.clone(),
                        item: item.id,
                    });
                }
            }
        }
        Ok(())
    }
}

fn item_declares_targets(payload: &serde_json::Value) -> bool {
    ["target_files", "expected_target_files"].iter().any(|key| {
        payload
            .get(*key)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|v| !v.trim().is_empty())
            })
    })
}
