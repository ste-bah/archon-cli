//! TASK-AGS-501: PipelinePattern — sequential agent chain adapter.
//!
//! Thin adapter that wraps a `PipelineEngineHandle` (phase-4 engine) so
//! sequential pipelines are addressable as a `Pattern`, enabling nesting
//! inside FanOut / Composite / Broker.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry};

// ---------------------------------------------------------------------------
// PipelineEngineHandle — slim trait wrapping the phase-4 engine
// ---------------------------------------------------------------------------

/// Slim trait so PipelinePattern can call the phase-4 engine without
/// depending on the full `archon-pipeline` crate directly.
#[async_trait]
pub trait PipelineEngineHandle: Send + Sync {
    /// Run a sequential pipeline of the given steps with the given input.
    /// Each step's output feeds into the next step's input.
    async fn run_steps(&self, steps: &[String], input: Value) -> Result<Value, String>;
}

// ---------------------------------------------------------------------------
// PipelineAdapterConfig
// ---------------------------------------------------------------------------

/// Configuration for the pipeline pattern adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineAdapterConfig {
    /// Ordered agent names forming the sequential chain.
    pub steps: Vec<String>,
    /// Whether to propagate intermediate errors or abort.
    #[serde(default = "default_true")]
    pub propagate_errors: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// PipelinePattern
// ---------------------------------------------------------------------------

/// Sequential agent chain adapter over the phase-4 pipeline engine.
///
/// Config is set at construction time — the steps are baked in so the
/// pattern can be registered once and invoked repeatedly.
pub struct PipelinePattern {
    engine: Arc<dyn PipelineEngineHandle>,
    config: PipelineAdapterConfig,
}

impl PipelinePattern {
    pub fn new(engine: Arc<dyn PipelineEngineHandle>, config: PipelineAdapterConfig) -> Self {
        Self { engine, config }
    }
}

#[async_trait]
impl Pattern for PipelinePattern {
    fn kind(&self) -> PatternKind {
        PatternKind::Pipeline
    }

    async fn execute(&self, input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
        self.engine
            .run_steps(&self.config.steps, input)
            .await
            .map_err(PatternError::Execution)
    }
}

/// Register a PipelinePattern into the registry under name "pipeline".
pub fn register(
    reg: &PatternRegistry,
    engine: Arc<dyn PipelineEngineHandle>,
    config: PipelineAdapterConfig,
) {
    reg.register("pipeline", Arc::new(PipelinePattern::new(engine, config)));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use serde_json::json;

    // Stub engine: runs each step by wrapping the value with the step name.
    // e.g., step "A" transforms {"v":"x"} into {"A": {"v":"x"}}
    struct StubEngine;

    #[async_trait]
    impl PipelineEngineHandle for StubEngine {
        async fn run_steps(&self, steps: &[String], input: Value) -> Result<Value, String> {
            let mut current = input;
            for step in steps {
                current = json!({ step.as_str(): current });
            }
            Ok(current)
        }
    }

    fn make_ctx() -> PatternCtx {
        use super::super::{PatternRegistry, TaskServiceHandle};

        struct DummyTaskService;

        #[async_trait]
        impl TaskServiceHandle for DummyTaskService {
            async fn submit(&self, _agent: &str, input: Value) -> Result<Value, PatternError> {
                Ok(input)
            }
        }

        PatternCtx {
            task_service: Arc::new(DummyTaskService),
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test".into(),
            deadline: None,
        }
    }

    #[tokio::test]
    async fn test_pipeline_three_steps_chain() {
        // TC-PAT-01: C(B(A(input))) — output of step N is input of step N+1.
        let engine = Arc::new(StubEngine);
        let config = PipelineAdapterConfig {
            steps: vec!["A".into(), "B".into(), "C".into()],
            propagate_errors: true,
        };
        let pattern = PipelinePattern::new(engine, config);
        let ctx = make_ctx();

        let input = json!("start");
        let result = pattern.execute(input, ctx).await.unwrap();

        // C(B(A("start"))) = {"C": {"B": {"A": "start"}}}
        assert_eq!(result, json!({"C": {"B": {"A": "start"}}}));
    }

    #[tokio::test]
    async fn test_pipeline_empty_steps_returns_input() {
        let engine = Arc::new(StubEngine);
        let config = PipelineAdapterConfig {
            steps: vec![],
            propagate_errors: true,
        };
        let pattern = PipelinePattern::new(engine, config);
        let ctx = make_ctx();

        let input = json!({"data": 42});
        let result = pattern.execute(input.clone(), ctx).await.unwrap();
        assert_eq!(result, input, "empty chain must be identity");
    }

    #[test]
    fn test_pipeline_adapter_config_serde_round_trip() {
        let cfg = PipelineAdapterConfig {
            steps: vec!["a".into(), "b".into(), "c".into()],
            propagate_errors: false,
        };

        let json_str = serde_json::to_string(&cfg).unwrap();
        let deserialized: PipelineAdapterConfig = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.steps, vec!["a", "b", "c"]);
        assert!(!deserialized.propagate_errors);
    }

    #[test]
    fn test_pipeline_pattern_kind() {
        let engine = Arc::new(StubEngine);
        let config = PipelineAdapterConfig {
            steps: vec![],
            propagate_errors: true,
        };
        let pattern = PipelinePattern::new(engine, config);
        assert!(matches!(pattern.kind(), PatternKind::Pipeline));
    }

    #[test]
    fn test_pipeline_register_and_resolve() {
        let reg = PatternRegistry::new();
        let engine: Arc<dyn PipelineEngineHandle> = Arc::new(StubEngine);
        let config = PipelineAdapterConfig {
            steps: vec!["x".into()],
            propagate_errors: true,
        };
        register(&reg, engine, config);

        let resolved = reg.resolve("pipeline");
        assert!(resolved.is_some());
        assert!(matches!(resolved.unwrap().kind(), PatternKind::Pipeline));
    }
}
