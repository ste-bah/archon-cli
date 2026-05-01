//! TASK-AGS-500: Foundation tests for Pattern trait, PatternRegistry, PatternSpec.
//!
//! These tests validate the public API surface before any concrete pattern
//! logic is implemented.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use archon_core::patterns::spec::{
    BrokerConfig, BrokerSelector, CircuitBreakerConfig, FanOutConfig, PatternSpec,
};
use archon_core::patterns::{
    Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry, TaskServiceHandle,
};

// ---------------------------------------------------------------------------
// Stub: NoopPattern for registry round-trip
// ---------------------------------------------------------------------------

struct NoopPattern;

#[async_trait]
impl Pattern for NoopPattern {
    fn kind(&self) -> PatternKind {
        PatternKind::Custom("noop".into())
    }

    async fn execute(&self, input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
        Ok(input)
    }
}

// ---------------------------------------------------------------------------
// Stub: TaskServiceHandle for PatternCtx construction
// ---------------------------------------------------------------------------

struct StubTaskService;

#[async_trait]
impl TaskServiceHandle for StubTaskService {
    async fn submit(&self, _agent: &str, input: Value) -> Result<Value, PatternError> {
        Ok(input)
    }
}

fn make_ctx() -> PatternCtx {
    PatternCtx {
        task_service: Arc::new(StubTaskService),
        registry: Arc::new(PatternRegistry::new()),
        trace_id: "test-trace".into(),
        deadline: None,
    }
}

// ---------------------------------------------------------------------------
// PatternRegistry round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_registry_register_and_resolve_round_trip() {
    let reg = PatternRegistry::new();
    let noop: Arc<dyn Pattern> = Arc::new(NoopPattern);

    reg.register("noop", noop.clone());

    let resolved = reg.resolve("noop");
    assert!(resolved.is_some(), "registered pattern must be resolvable");
}

#[test]
fn test_registry_resolve_missing_returns_none() {
    let reg = PatternRegistry::new();
    assert!(reg.resolve("nonexistent").is_none());
}

#[test]
fn test_registry_list_names() {
    let reg = PatternRegistry::new();
    reg.register("alpha", Arc::new(NoopPattern));
    reg.register("beta", Arc::new(NoopPattern));

    let mut names = reg.list_names();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}

// ---------------------------------------------------------------------------
// PatternSpec::is_supported version check
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_spec_is_supported_current_version() {
    let spec = PatternSpec {
        pattern_version: 5,
        ..PatternSpec::default()
    };
    assert!(spec.is_supported(5), "current version must be supported");
}

#[test]
fn test_pattern_spec_is_supported_n_minus_1() {
    let spec = PatternSpec {
        pattern_version: 4,
        ..PatternSpec::default()
    };
    assert!(spec.is_supported(5), "N-1 version must be supported");
}

#[test]
fn test_pattern_spec_is_supported_rejects_n_minus_2() {
    let spec = PatternSpec {
        pattern_version: 3,
        ..PatternSpec::default()
    };
    assert!(!spec.is_supported(5), "N-2 version must be rejected");
}

#[test]
fn test_pattern_spec_is_supported_rejects_n_plus_1() {
    let spec = PatternSpec {
        pattern_version: 6,
        ..PatternSpec::default()
    };
    assert!(!spec.is_supported(5), "N+1 version must be rejected");
}

// ---------------------------------------------------------------------------
// PatternError variant coverage
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_error_variants_exist() {
    // Verify all 7 required variants compile and display correctly.
    let errors: Vec<PatternError> = vec![
        PatternError::Timeout,
        PatternError::CircuitOpen {
            name: "agent-x".into(),
        },
        PatternError::RemoteUnreachable {
            url: "http://localhost".into(),
            cause: "connection refused".into(),
        },
        PatternError::CompositeCycle {
            path: vec!["A".into(), "B".into(), "A".into()],
        },
        PatternError::BrokerNoCandidate {
            reasons: vec!["agent-a: unavailable".into()],
        },
        PatternError::PartialResult {
            merged: json!({"partial": true}),
            errors: vec!["worker-2 failed".into()],
        },
        PatternError::Execution("something went wrong".into()),
    ];

    // Each variant must produce a non-empty Display string.
    for e in &errors {
        let msg = format!("{e}");
        assert!(!msg.is_empty(), "PatternError Display must not be empty");
    }

    assert_eq!(errors.len(), 7, "must cover all 7 PatternError variants");
}

// ---------------------------------------------------------------------------
// PatternKind variants
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_kind_variants() {
    let kinds = [
        PatternKind::Pipeline,
        PatternKind::FanOut,
        PatternKind::Broker,
        PatternKind::Composite,
        PatternKind::Remote,
        PatternKind::Custom("my-custom".into()),
    ];
    assert_eq!(kinds.len(), 6);
}

// ---------------------------------------------------------------------------
// NoopPattern execute round-trip via Pattern trait
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_noop_pattern_execute_returns_input() {
    let noop = NoopPattern;
    let input = json!({"hello": "world"});
    let ctx = make_ctx();

    let result = noop.execute(input.clone(), ctx).await.unwrap();
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// CircuitBreakerConfig defaults
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_config_defaults() {
    let cfg = CircuitBreakerConfig::default();
    assert_eq!(cfg.failure_threshold, 3);
    assert_eq!(cfg.reset_after, Duration::from_secs(60));
    assert_eq!(cfg.half_open_probes, 1);
}

// ---------------------------------------------------------------------------
// PatternSpec default timeout is 30 minutes (REQ-ARCH-006)
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_spec_default_timeout_30_minutes() {
    let spec = PatternSpec::default();
    assert_eq!(spec.timeout, Duration::from_secs(30 * 60));
}

// ---------------------------------------------------------------------------
// Serde round-trips for spec types
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_spec_serde_round_trip() {
    let spec = PatternSpec {
        pattern_version: 2,
        kind: PatternKind::Pipeline,
        config: json!({"steps": ["a", "b"]}),
        timeout: Duration::from_secs(120),
        circuit_breaker: CircuitBreakerConfig::default(),
    };

    let json_str = serde_json::to_string(&spec).unwrap();
    let deserialized: PatternSpec = serde_json::from_str(&json_str).unwrap();

    assert_eq!(deserialized.pattern_version, 2);
    assert_eq!(deserialized.timeout, Duration::from_secs(120));
}

#[test]
fn test_fanout_config_serde_round_trip() {
    let cfg = FanOutConfig {
        workers: vec!["w1".into(), "w2".into()],
        aggregator: "agg".into(),
        partition_fn: Some("split_by_id".into()),
    };

    let json_str = serde_json::to_string(&cfg).unwrap();
    let deserialized: FanOutConfig = serde_json::from_str(&json_str).unwrap();

    assert_eq!(deserialized.workers, vec!["w1", "w2"]);
    assert_eq!(deserialized.aggregator, "agg");
    assert_eq!(deserialized.partition_fn, Some("split_by_id".into()));
}

#[test]
fn test_broker_config_serde_round_trip() {
    let cfg = BrokerConfig {
        candidates: vec!["c1".into(), "c2".into()],
        selector: BrokerSelector::Capability,
    };

    let json_str = serde_json::to_string(&cfg).unwrap();
    let deserialized: BrokerConfig = serde_json::from_str(&json_str).unwrap();

    assert_eq!(deserialized.candidates, vec!["c1", "c2"]);
    assert!(matches!(deserialized.selector, BrokerSelector::Capability));
}
