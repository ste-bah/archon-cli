use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::WorkflowResult;
use crate::spec::{ProviderTier, StageKind};
use crate::write_coordinator::WriteBoundaryProbe;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRunRequest {
    pub run_id: String,
    pub stage_id: String,
    pub stage_kind: StageKind,
    pub agent: Option<String>,
    pub task: String,
    pub attempt: u32,
    pub provider_tier: ProviderTier,
    pub depends_on: Vec<String>,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRunOutput {
    pub body: String,
    pub extension: String,
    pub provider_id: Option<String>,
    pub resolved_model: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tool_uses: Vec<Value>,
}

impl StageRunOutput {
    pub fn markdown(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            extension: "md".into(),
            provider_id: None,
            resolved_model: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            tool_uses: Vec::new(),
        }
    }
}

#[cfg(test)]
mod usage_serialization_tests {
    use super::StageRunOutput;

    #[test]
    fn unknown_stage_usage_serializes_as_null() {
        let value = serde_json::to_value(StageRunOutput::markdown("result"))
            .expect("stage output serializes");

        assert!(value["tokens_in"].is_null());
        assert!(value["tokens_out"].is_null());
        assert!(value["cost_usd"].is_null());
    }

    #[test]
    fn legacy_stage_outputs_with_numeric_or_missing_usage_deserialize() {
        let numeric: StageRunOutput = serde_json::from_value(serde_json::json!({
            "body": "result",
            "extension": "md",
            "provider_id": null,
            "resolved_model": null,
            "tokens_in": 12,
            "tokens_out": 3,
            "cost_usd": 0.0,
            "tool_uses": []
        }))
        .expect("legacy numeric usage");
        assert_eq!(numeric.tokens_in, Some(12));
        assert_eq!(numeric.tokens_out, Some(3));

        let missing: StageRunOutput = serde_json::from_value(serde_json::json!({
            "body": "result",
            "extension": "md",
            "provider_id": null,
            "resolved_model": null,
            "tool_uses": []
        }))
        .expect("legacy missing usage");
        assert_eq!(missing.tokens_in, None);
        assert_eq!(missing.tokens_out, None);
        assert_eq!(missing.cost_usd, None);
    }
}

#[async_trait]
pub trait WorkflowStageRunner: Send + Sync + WriteBoundaryProbe {
    async fn run_stage(&self, request: StageRunRequest) -> WorkflowResult<StageRunOutput>;

    /// Maximum number of stages this runner can execute concurrently, when the
    /// runner is backed by a bounded resource (e.g. a subagent manager with a
    /// hard concurrency cap that *rejects* — rather than queues — overflow).
    ///
    /// Fan-out scheduling clamps its semaphore width to this value so the
    /// number of in-flight items never exceeds what the runner can accept.
    /// Returning `None` (the default) means "no runner-imposed limit"; only the
    /// spec/policy `max_parallelism` applies.
    fn max_concurrency(&self) -> Option<usize> {
        None
    }
}
